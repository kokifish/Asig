//! Claude Code 的会话状态监控实现。
//!
//! 文件结构:`~/.claude/sessions/<pid>.json`,status: "busy" | "idle" | "shell" | "waiting",
//! 配合 transcript(`~/.claude/projects/*/<sessionId>.jsonl`)尾部信号判 NeedsDeci。
//!
//! CodeBuddy 曾与本实现共用(ClaudeLikeSource 参数化 root),现已暂不支持;
//! `codebuddy()` 构造已移除,参数化结构(root 字段)保留供未来恢复。
//!
//! **按 cwd 聚合**:同目录下的多个 session(用户手开的 interactive + claude
//! `--fork-session` 派发的后台子 claude `kind:"bg"`)合并为**一个**会话 —— interactive
//! 作主,bg 不单独显示,但其 busy 活跃度合并进主会话状态。否则把任务 fork 到后台跑时,
//! 主进程会 idle 成 shell,Asig 会误判为不在运行。纯 bg 无 interactive 的目录跳过(避免
//! 与 OpenClaw source 重叠)。
//!
//! Offline 检测(廉价、可靠):
//!   - `status` 字段只有 busy/idle/shell/waiting,没有 error/offline;
//!   - `statusUpdatedAt` 实测只在**状态转换**时写,不是周期心跳(busy 会话跑很久也
//!     不更新),故**不能**用心跳新鲜度判"卡死"——会误报长任务;
//!   - 可靠信号:进程死了。Claude 干净退出会清掉 session 文件;**残留的死 pid 文件
//!     = 崩溃/被杀**。Asig 只对"本轮之前见过它活着"的会话报 Offline,过滤掉古老残留。
//!
//! NeedsDeci(待决策)检测:
//!   - `status == "waiting"`(Claude 等用户输入/授权,如工具 permission)→ 直接 NeedsDeci;
//!   - 否则 session 文件的 `status` 在"Claude 问你问题"时仍是 busy(turn 还没结束),
//!     故单看 busy/idle 只能区分 Working/Done,得到 NeedsDeci 靠下面的 transcript 信号。
//!   - 真正信号在会话 transcript(`~/.claude/projects/*/<sessionId>.jsonl`)尾部最后一条
//!     有意义事件:busy 且 `end_turn`(模型说完、把控制权交还用户)→ NeedsDeci(等你
//!     输入/决策);`user`(用户刚输入、Claude 正在处理)/`tool_use`/未知 → Working。
//!     关键:end_turn 之后若已有 user 消息,判 Working 而非残留 end_turn 误判 NeedsDeci
//!     (用户回了 = Claude 在跑,不是等你)。只读文件尾部 ~16KB,3s 一次轮询开销可忽略;
//!     读不到 transcript → 回退 Working(不报错)。

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// ~/.claude/sessions/<pid>.json 的结构(实测,版本 2.1.x;字段 camelCase)。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    pid: u32,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    /// `"interactive"`(用户手动 REPL)/ `"bg"`(claude `--fork-session` 派发的后台子 claude)。
    /// bg 不单独显示,但其 busy 活跃度合并进**同 cwd 的 interactive 主会话** —— 否则 fork
    /// 任务到后台跑时主进程 idle 成 shell,Asig 会误判不在运行。纯 bg 无 interactive 的目录
    /// 整组跳过(避免与 OpenClaw source 重叠)。无此字段则为 None(普通 interactive 会话)。
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>, // "busy" | "idle" | "shell" | "waiting"
    /// status 最后更新时间(epoch ms)。Claude 只在状态转换时写(非心跳),用作同 cwd 多个
    /// interactive 会话时挑「最新活动」者作主(= 用户当前在用的),避免遗留 busy+end_turn
    /// REPL 把整个组拉成 NeedsDeci。旧文件缺该字段 → 0。
    #[serde(default)]
    status_updated_at: u64,
}

pub struct ClaudeLikeSource {
    pub kind: AgentKind,
    pub root: PathBuf,
    /// pid -> 上次见到的状态。跨轮询保留,用于识别「曾经活着、现在失联」的会话。
    seen: Mutex<HashMap<u32, AgentStatus>>,
}

impl ClaudeLikeSource {
    pub fn claude() -> Option<Self> {
        Some(Self {
            kind: AgentKind::Claude,
            root: dirs::home_dir()?.join(".claude"),
            seen: Mutex::new(HashMap::new()),
        })
    }
}

/// 读 `<root>/sessions/*.json`,逐个解析为 `SessionFile`(打不开/解析失败跳过)。无目录 → 空。
/// `discover` 与 `probe` 共用,避免读文件逻辑两处分写。
fn read_session_files(root: &Path) -> Vec<SessionFile> {
    let Ok(entries) = std::fs::read_dir(root.join("sessions")) else {
        return Vec::new(); // 没装该工具 → 空目录 → 无会话
    };
    entries
        .flatten()
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str::<SessionFile>(&text).ok()
        })
        .collect()
}

impl AgentSource for ClaudeLikeSource {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    fn discover(&self) -> Vec<AgentSession> {
        let files = read_session_files(&self.root);
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let root = &self.root;
        discover_from(
            &files,
            &mut seen,
            crate::sys::pid_alive,
            |sid| last_signal(root, sid),
            self.kind,
        )
    }
}

/// 纯函数核心:给定本轮发现的文件集 + 历史可见状态 + 存活判定 + 尾部信号探测,决定
/// 每个会话的状态,并更新 `seen`。文件系统 / pid / 时间 / transcript 都被抽掉,便于 MOCK。
///
/// **按 cwd 聚合**:同目录下的多个 session(用户手开的 interactive + claude `--fork-session`
/// 派发的 bg 子进程)合并为**一个**会话 —— interactive 作主(标识/cwd/sessionId),bg 不单独
/// 显示,但其 busy 活跃度合并进主会话状态(取组内最活跃)。否则 fork 任务到后台跑时,主进程
/// idle 成 shell,会被误判为不在运行。纯 bg 无 interactive 的目录整组跳过(避免与 OpenClaw
/// source 重叠)。`seen` 只记每个主(interactive)pid;本轮消失的主 pid 被自然裁掉。
fn discover_from(
    files: &[SessionFile],
    seen: &mut HashMap<u32, AgentStatus>,
    is_alive: impl Fn(u32) -> bool,
    signal_of: impl Fn(&str) -> Option<String>,
    kind: AgentKind,
) -> Vec<AgentSession> {
    // 按目录分组,记下首次出现顺序(稳定输出)。
    let mut groups: HashMap<Option<&str>, Vec<&SessionFile>> = HashMap::new();
    let mut order: Vec<Option<&str>> = Vec::new();
    for f in files {
        let c = f.cwd.as_deref();
        if !groups.contains_key(&c) {
            order.push(c);
        }
        groups.entry(c).or_default().push(f);
    }

    let mut live: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for cwd in order {
        let group = &groups[&cwd];
        // 主 = 组内最新活动的 interactive(status_updated_at 最大 = 用户当前在用的)。
        // 同 cwd 多个 interactive 时取最新,旧的视为遗留 REPL 不作主(否则它会经 group_status
        // 的活跃度合并把整组拉成 NeedsDeci)。纯 bg 无 interactive 主 → 跳过整组(避免与
        // OpenClaw source 重叠)。
        let Some(primary) = group
            .iter()
            .filter(|f| f.kind.as_deref() != Some("bg"))
            .max_by_key(|f| f.status_updated_at)
            .copied()
        else {
            continue;
        };
        let prev = seen.get(&primary.pid).copied();
        let Some(st) = group_status(primary, group, prev, &is_alive, &signal_of) else {
            continue;
        };
        seen.insert(primary.pid, st);
        live.insert(primary.pid);
        out.push(AgentSession {
            kind,
            id: format!("{:?}:{}", kind, primary.pid),
            native_id: primary.pid.to_string(),
            cwd: primary.cwd.clone().map(PathBuf::from),
            status: st,
            label: primary.session_id.clone(),
        });
    }
    // 本轮没出现的(主)pid → 不再盯。干净退出就这样被自然忘掉。
    seen.retain(|pid, _| live.contains(pid));
    out
}

/// 组内聚合状态:对 **primary 与 bg 子进程**调 `classify`,取**最活跃**的(NeedsDeci > Working >
/// Done > Offline)。bg 子进程的 busy 据此贡献给主会话。其他 interactive(用户另开的独立
/// REPL)被跳过,不污染主状态 —— 详见函数体内的过滤。
fn group_status(
    primary: &SessionFile,
    group: &[&SessionFile],
    prev_of_primary: Option<AgentStatus>,
    is_alive: &impl Fn(u32) -> bool,
    signal_of: &impl Fn(&str) -> Option<String>,
) -> Option<AgentStatus> {
    let mut best: Option<AgentStatus> = None;
    for &f in group {
        // 只聚合 primary 与 bg 子进程;**其他 interactive 是用户另开的独立 REPL,跳过**
        // (否则同 cwd 一个遗留 busy+end_turn 会话会把整组拉成 NeedsDeci)。bg 子进程的
        // busy 活跃度仍合并进主(fork 任务后台跑时主 idle 成 shell 不被误判不在运行)。
        let is_bg = f.kind.as_deref() == Some("bg");
        if f.pid != primary.pid && !is_bg {
            continue;
        }
        let prev = if f.pid == primary.pid {
            prev_of_primary
        } else {
            None
        };
        let alive = is_alive(f.pid);
        // 只对 busy 进程读 transcript(idle/shell→Done 无需、省一次文件读)。
        let sig = if alive && f.status.as_deref() == Some("busy") {
            f.session_id.as_deref().and_then(signal_of)
        } else {
            None
        };
        let Some(st) = classify(f, prev, alive, sig.as_deref()) else {
            continue;
        };
        best = Some(most_active(best, st));
    }
    best
}

/// 同 cwd 内活跃度排序:NeedsDeci/Error > Working > Done > Offline;返回更活跃者。
/// 与 `AgentStatus::global_priority` 故意不同 —— 此处 Offline 视为最不活跃(崩溃的 bg 子进程
/// 不该把整个 agent 拉成 Offline),后者 Offline 优先级最高(全局该报异常)。
fn most_active(a: Option<AgentStatus>, b: AgentStatus) -> AgentStatus {
    fn liveness_rank(st: AgentStatus) -> u8 {
        match st {
            AgentStatus::NeedsDeci => 4,
            AgentStatus::Error => 4, // 出错也需关注;Claude source 不产生(OpenClaw 才有)
            AgentStatus::Working => 3,
            AgentStatus::Done => 2,
            AgentStatus::Offline => 1,
        }
    }
    match a {
        Some(prev) if liveness_rank(prev) >= liveness_rank(b) => prev,
        _ => b,
    }
}

/// 单文件状态判定(纯函数)。
///
/// - pid 活且 `idle`/`shell` → Done;
/// - pid 活且 `waiting`(Claude 等用户输入/授权,如工具 permission)→ NeedsDeci
///   (status 层明确,优先于 transcript——此时尾部可能是历史 tool_use);
/// - pid 活且 `busy`:`signal == "end_turn"`(模型说完、等用户回)→ NeedsDeci;
///   `signal` 为 `"user"`(用户刚输入、Claude 正在处理)/`tool_use`/未知/读不到 → Working;
/// - pid 活、status 未知 → Working;
/// - pid 死且 `seen` 里曾见过(活的)→ **Offline**(崩溃/被杀,文件残留);
/// - pid 死且从没见过 → `None`(古老残留,跳过,不制造噪音)。
fn classify(
    f: &SessionFile,
    prev: Option<AgentStatus>,
    alive: bool,
    signal: Option<&str>,
) -> Option<AgentStatus> {
    if alive {
        Some(match f.status.as_deref() {
            // idle/shell = 空闲(shell=Claude REPL 等输入,无活跃任务)→ Done,非 Working。
            Some("idle") | Some("shell") => AgentStatus::Done,
            // waiting = Claude 等用户输入/授权(如工具 permission),明确的 NeedsDeci,
            // 优先于 transcript(尾部可能是历史 tool_use,但 status 已切 waiting)。
            Some("waiting") => AgentStatus::NeedsDeci,
            Some("busy") => match signal {
                Some("end_turn") => AgentStatus::NeedsDeci,
                _ => AgentStatus::Working, // user / tool_use / 未知 / 读不到 → 正在跑
            },
            _ => AgentStatus::Working,
        })
    } else {
        prev.map(|_| AgentStatus::Offline)
    }
}

/// 读会话 transcript(`<root>/projects/*/<sessionId>.jsonl`)尾部的"最后信号"。
/// busy 会话据此区分 NeedsDeci(end_turn)vs Working(其他)。读不到 → None(回退 Working)。
fn last_signal(root: &Path, session_id: &str) -> Option<String> {
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return None;
    };
    for e in entries.flatten() {
        let p = e.path().join(format!("{session_id}.jsonl"));
        if p.is_file() {
            return read_tail_signal(&p);
        }
    }
    None
}

/// 只读文件尾部 ~16KB,反序找最后一条**有意义事件**:`type:"user"`(用户刚输入,Claude
/// 正在处理)→ 返回 `"user"`;`type:"assistant"` → 返回其 `message.stop_reason`
/// (`end_turn`/`tool_use`/...)。这样 end_turn 之后若已有 user 消息,判 Working 而非
/// 残留 end_turn 误判 NeedsDeci。尾部 I/O 走共用 `jsonl_tail::read_tail_lines`。
fn read_tail_signal(path: &Path) -> Option<String> {
    let events = crate::jsonl_tail::read_tail_lines(path, 16_384)?;
    for v in events.iter().rev() {
        let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
            continue;
        };
        if ty == "user" {
            return Some("user".to_string());
        }
        if ty == "assistant" {
            if let Some(sr) = v
                .get("message")
                .and_then(|m| m.get("stop_reason"))
                .and_then(|s| s.as_str())
            {
                return Some(sr.to_string());
            }
        }
    }
    None
}

// ---- CLI 探针(`probe-claude`):复用 discover 的聚合 + classify,输出诊断视图 ----

/// 一个 session 文件的诊断视图(probe 用)。
pub struct ClaudeMember {
    pub pid: u32,
    pub kind: Option<String>,
    /// session 文件的 status:busy / idle / shell / waiting。
    pub status_field: Option<String>,
    /// status 最后更新距 now 的秒数;缺字段/未知 = -1。
    pub age_s: i64,
    pub alive: bool,
    pub session_id: Option<String>,
    /// transcript 尾部信号(busy 才读):end_turn / user / tool_use / None。
    pub signal: Option<String>,
    /// 单成员 classify(probe 无 seen → 死 pid 按「古老残留」跳过 = None,不报 Offline)。
    pub classify: Option<AgentStatus>,
    pub is_primary: bool,
    /// 是否参与组聚合(primary 或 bg);其他 interactive = false(方案 A:不合并)。
    pub merged: bool,
}

/// 同 cwd 一组的诊断视图。组状态 = `group_status`(primary + bg);其他 interactive 展示但不合并。
pub struct ClaudeProbe {
    pub cwd: Option<String>,
    pub primary_pid: u32,
    pub status: AgentStatus,
    pub members: Vec<ClaudeMember>,
}

/// 探针:读真实 ~/.claude,按 cwd 聚合,每组建 primary + 组状态 + 成员诊断(供 CLI
/// `probe-claude`)。复用 `discover_from` 的聚合选择 + `classify`/`group_status`(单一判定源)。
/// probe 无状态(seen=None)→ 死 pid 按「古老残留」跳过(classify=None),不报 Offline;
/// 诊断 Offline 需看运行中的 Asig(probe 不跟踪 seen 历史)。
pub fn probe() -> Vec<ClaudeProbe> {
    let Some(src) = ClaudeLikeSource::claude() else {
        return Vec::new();
    };
    let files = read_session_files(&src.root);
    // 按 cwd 聚合(同 discover_from),保持首次出现顺序。
    let mut groups: HashMap<Option<&str>, Vec<&SessionFile>> = HashMap::new();
    let mut order: Vec<Option<&str>> = Vec::new();
    for f in &files {
        let c = f.cwd.as_deref();
        if !groups.contains_key(&c) {
            order.push(c);
        }
        groups.entry(c).or_default().push(f);
    }
    let now_ms = crate::sys::now_ms();
    let mut out = Vec::new();
    for cwd in order {
        let group = &groups[&cwd];
        let Some(primary) = group
            .iter()
            .filter(|f| f.kind.as_deref() != Some("bg"))
            .max_by_key(|f| f.status_updated_at)
            .copied()
        else {
            continue;
        };
        let status = group_status(primary, group, None, &crate::sys::pid_alive, &|sid| {
            last_signal(&src.root, sid)
        })
        .unwrap_or(AgentStatus::Done);
        let members = group
            .iter()
            .map(|&f| {
                let alive = crate::sys::pid_alive(f.pid);
                let is_primary = f.pid == primary.pid;
                let merged = is_primary || f.kind.as_deref() == Some("bg");
                let signal = if alive && f.status.as_deref() == Some("busy") {
                    f.session_id
                        .as_deref()
                        .and_then(|sid| last_signal(&src.root, sid))
                } else {
                    None
                };
                let age_s = if now_ms >= f.status_updated_at && f.status_updated_at > 0 {
                    ((now_ms - f.status_updated_at) / 1000) as i64
                } else {
                    -1
                };
                let classify_val = classify(f, None, alive, signal.as_deref());
                ClaudeMember {
                    pid: f.pid,
                    kind: f.kind.clone(),
                    status_field: f.status.clone(),
                    age_s,
                    alive,
                    session_id: f.session_id.clone(),
                    signal,
                    classify: classify_val,
                    is_primary,
                    merged,
                }
            })
            .collect();
        out.push(ClaudeProbe {
            cwd: cwd.map(String::from),
            primary_pid: primary.pid,
            status,
            members,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonl_tail::write_tmp;

    fn pf(pid: u32, status: Option<&str>) -> SessionFile {
        pf_cwd(pid, status, None)
    }

    /// 带 cwd 的构造(不同 cwd = 不同会话,用以测试聚合边界)。
    fn pf_cwd(pid: u32, status: Option<&str>, cwd: Option<&str>) -> SessionFile {
        SessionFile {
            pid,
            session_id: Some(format!("s{pid}")), // 有 session_id 才会触发 transcript 读取
            cwd: cwd.map(str::to_string),
            kind: None,
            status: status.map(str::to_string),
            status_updated_at: 0,
        }
    }

    // ---- classify:纯函数 ----

    #[test]
    fn classify_alive_maps_status() {
        // busy + 无 stop_reason(读不到 transcript)→ Working
        assert_eq!(
            classify(&pf(1, Some("busy")), None, true, None),
            Some(AgentStatus::Working)
        );
        // busy + tool_use → Working(正在跑工具)
        assert_eq!(
            classify(&pf(1, Some("busy")), None, true, Some("tool_use")),
            Some(AgentStatus::Working)
        );
        // busy + end_turn → NeedsDeci(等用户回)← bug 修复的核心
        assert_eq!(
            classify(&pf(1, Some("busy")), None, true, Some("end_turn")),
            Some(AgentStatus::NeedsDeci)
        );
        // busy + user(用户刚输入、Claude 处理中)→ Working(曾因残留 end_turn 误判 NeedsDeci)
        assert_eq!(
            classify(&pf(1, Some("busy")), None, true, Some("user")),
            Some(AgentStatus::Working)
        );
        // waiting(Claude 等用户输入/授权)→ NeedsDeci,优先于 transcript(尾部可能是历史 tool_use)
        assert_eq!(
            classify(&pf(1, Some("waiting")), None, true, Some("tool_use")),
            Some(AgentStatus::NeedsDeci)
        );
        assert_eq!(
            classify(&pf(1, Some("waiting")), None, true, None),
            Some(AgentStatus::NeedsDeci)
        );
        // idle → Done(stop_reason 无关;即 idle 优先于 stop_reason)
        assert_eq!(
            classify(&pf(1, Some("idle")), None, true, Some("end_turn")),
            Some(AgentStatus::Done)
        );
        assert_eq!(
            classify(&pf(1, Some("idle")), None, true, None),
            Some(AgentStatus::Done)
        );
        // shell(Claude REPL 模式,空闲等输入)→ Done,非 Working(曾误判运行中)
        assert_eq!(
            classify(&pf(1, Some("shell")), None, true, None),
            Some(AgentStatus::Done)
        );
        // status 未知 → Working
        assert_eq!(
            classify(&pf(1, None), None, true, None),
            Some(AgentStatus::Working)
        );
        assert_eq!(
            classify(&pf(1, Some("wat")), None, true, None),
            Some(AgentStatus::Working)
        );
    }

    #[test]
    fn classify_dead_seen_before_is_offline() {
        // 曾见过(活的)→ 现在死了 = 失联
        assert_eq!(
            classify(
                &pf(1, Some("busy")),
                Some(AgentStatus::Working),
                false,
                None
            ),
            Some(AgentStatus::Offline)
        );
        assert_eq!(
            classify(&pf(1, Some("idle")), Some(AgentStatus::Done), false, None),
            Some(AgentStatus::Offline)
        );
        // 上一轮就已经是 Offline,文件还残留 → 继续 Offline
        assert_eq!(
            classify(
                &pf(1, Some("busy")),
                Some(AgentStatus::Offline),
                false,
                None
            ),
            Some(AgentStatus::Offline)
        );
    }

    // ---- read_tail_signal:transcript 尾部信号 ----

    #[test]
    fn read_tail_signal_user_after_end_turn_is_user() {
        // end_turn 后有 user(用户回了)→ "user"(Claude 处理中 → Working),不误判残留 end_turn
        let p = write_tmp(
            "user_after_end",
            &[
                r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
                r#"{"type":"user","message":{"role":"user"}}"#,
            ],
        );
        assert_eq!(read_tail_signal(&p).as_deref(), Some("user"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_signal_end_turn_when_last_is_end_turn() {
        // 最后是 assistant end_turn(等用户)→ "end_turn" → NeedsDeci
        let p = write_tmp(
            "end_last",
            &[
                r#"{"type":"user","message":{"role":"user"}}"#,
                r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
            ],
        );
        assert_eq!(read_tail_signal(&p).as_deref(), Some("end_turn"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_signal_tool_use_is_tool_use() {
        let p = write_tmp(
            "tool",
            &[r#"{"type":"assistant","message":{"stop_reason":"tool_use"}}"#],
        );
        assert_eq!(read_tail_signal(&p).as_deref(), Some("tool_use"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn classify_dead_never_seen_is_skipped() {
        // 古老残留 → None(不报)
        assert_eq!(classify(&pf(1, Some("busy")), None, false, None), None);
    }

    // ---- most_active:聚合活跃度 ----

    #[test]
    fn most_active_picks_busier() {
        use AgentStatus::*;
        assert_eq!(most_active(Some(Working), Done), Working);
        assert_eq!(most_active(Some(Done), Working), Working);
        assert_eq!(most_active(Some(NeedsDeci), Working), NeedsDeci);
        assert_eq!(most_active(Some(Working), NeedsDeci), NeedsDeci);
        assert_eq!(most_active(Some(Offline), Working), Working);
        assert_eq!(most_active(None, Done), Done);
    }

    // ---- discover_from:MOCK(is_alive / stop_reason / files / seen 全注入)----

    #[test]
    fn discover_healthy_working() {
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(seen.get(&100), Some(&AgentStatus::Working));
    }

    #[test]
    fn discover_busy_end_turn_is_needs_deci() {
        // busy 且 transcript 最后一条 end_turn → NeedsDeci(等用户)
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| true,
            |_| Some("end_turn".into()),
            AgentKind::Claude,
        );
        assert_eq!(out[0].status, AgentStatus::NeedsDeci);
        assert_eq!(seen.get(&100), Some(&AgentStatus::NeedsDeci));
    }

    #[test]
    fn discover_busy_tool_use_is_working() {
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| true,
            |_| Some("tool_use".into()),
            AgentKind::Claude,
        );
        assert_eq!(out[0].status, AgentStatus::Working);
    }

    #[test]
    fn discover_idle_never_reads_transcript() {
        // idle → Done;stop_reason_of 即使会 panic 也不该被调用(传一个必崩闭包验证)
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(100, Some("idle"))],
            &mut seen,
            |_| true,
            |_| panic!("idle 不该读 transcript"),
            AgentKind::Claude,
        );
        assert_eq!(out[0].status, AgentStatus::Done);
    }

    #[test]
    fn discover_dead_seen_before_becomes_offline() {
        // 上一轮见过 100 在 Working;本轮 pid 死了 → Offline
        let mut seen = HashMap::from([(100, AgentStatus::Working)]);
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| false,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, AgentStatus::Offline);
        assert_eq!(seen.get(&100), Some(&AgentStatus::Offline));
    }

    #[test]
    fn discover_ancient_leftover_is_ignored() {
        // 从没见过的死 pid 文件 → 不报,seen 也不记
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(999, Some("busy"))],
            &mut seen,
            |_| false,
            |_| None,
            AgentKind::Claude,
        );
        assert!(out.is_empty());
        assert!(seen.is_empty());
    }

    #[test]
    fn discover_offline_recovers_to_working() {
        // 曾 Offline;进程复活且 busy → Working
        let mut seen = HashMap::from([(300, AgentStatus::Offline)]);
        let out = discover_from(
            &[pf(300, Some("busy"))],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(seen.get(&300), Some(&AgentStatus::Working));
    }

    #[test]
    fn discover_prunes_vanished_pids() {
        // 上轮见过 100、777;本轮只剩 100 的文件 → 777 被裁掉(干净退出)
        let mut seen = HashMap::from([(100, AgentStatus::Working), (777, AgentStatus::Done)]);
        let _ = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(seen.len(), 1);
        assert!(seen.contains_key(&100));
        assert!(!seen.contains_key(&777));
    }

    #[test]
    fn discover_mixed_alive_and_dead() {
        // 不同目录 = 不同会话:100 活着 busy;200 上轮见过、现在死了 → 一 Working 一 Offline
        let mut seen = HashMap::from([(200, AgentStatus::Working)]);
        let out = discover_from(
            &[
                pf_cwd(100, Some("busy"), Some("/a")),
                pf_cwd(200, Some("busy"), Some("/b")),
            ],
            &mut seen,
            |pid| pid == 100,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(out[1].status, AgentStatus::Offline);
    }

    // ---- 按 cwd 聚合(interactive + bg 子进程)----

    #[test]
    fn discover_bg_merges_into_interactive_same_cwd() {
        // 同 cwd:interactive + bg 合并为 1 个,主 = interactive(200);bg(100)不单独显示、不进 seen。
        let mut seen = HashMap::new();
        let mut bg = pf(100, Some("busy"));
        bg.kind = Some("bg".into());
        let out = discover_from(
            &[bg, pf(200, Some("busy"))],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1, "同 cwd 合并为 1 个");
        assert_eq!(out[0].native_id, "200", "主 = interactive");
        assert!(!seen.contains_key(&100), "bg pid 不进 seen");
        assert!(seen.contains_key(&200));
    }

    #[test]
    fn discover_bg_busy_lifts_interactive_shell_to_working() {
        // fork 任务到后台跑的典型场景:interactive idle 成 shell(单独=Done)、bg busy(单独=Working)
        // → 同 cwd 聚合为 1 个,状态 = Working(取组内最活跃)← 本次 bug 的核心修复。
        let mut seen = HashMap::new();
        let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
        bg.kind = Some("bg".into());
        bg.session_id = None; // bg 即便没 sessionId 也贡献 busy 活跃度
        let mut inter = pf_cwd(200, Some("shell"), Some("/a"));
        inter.kind = Some("interactive".into());
        let out = discover_from(
            &[bg, inter],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].native_id, "200");
        assert_eq!(
            out[0].status,
            AgentStatus::Working,
            "bg busy 把 shell 主提升为 Working"
        );
        assert!(!seen.contains_key(&100));
        assert!(seen.contains_key(&200));
    }

    #[test]
    fn discover_pure_bg_group_without_interactive_is_skipped() {
        // 组里只有 bg(无 interactive 主)→ 整组跳过(避免与 OpenClaw source 重叠)。
        let mut seen = HashMap::new();
        let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
        bg.kind = Some("bg".into());
        let out = discover_from(&[bg], &mut seen, |_| true, |_| None, AgentKind::Claude);
        assert!(out.is_empty());
        assert!(seen.is_empty());
    }

    #[test]
    fn discover_distinct_cwd_are_distinct_sessions() {
        // 不同 cwd = 不同会话,各聚合成 1 个。
        let mut seen = HashMap::new();
        let out = discover_from(
            &[
                pf_cwd(100, Some("busy"), Some("/a")),
                pf_cwd(200, Some("busy"), Some("/b")),
            ],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].cwd.as_deref(), Some(Path::new("/a")));
        assert_eq!(out[1].cwd.as_deref(), Some(Path::new("/b")));
    }

    // ---- 多 interactive 同 cwd:取最新,旧的忽略(方案 A 修复)----

    #[test]
    fn discover_multiple_interactive_picks_newest_and_ignores_stale() {
        // 同 cwd 两个 interactive:旧的 busy+end_turn(单独会判 NeedsDeci)、新的 idle(最新)。
        // 修复前:most_active 合并 → NeedsDeci(旧污染)。
        // 修复后:primary = 最新 idle → Done;旧的 interactive 不参与合并。
        let mut old = pf_cwd(72955, Some("busy"), Some("/a"));
        old.status_updated_at = 1000;
        old.session_id = Some("old".into());
        let mut new = pf_cwd(61965, Some("idle"), Some("/a"));
        new.status_updated_at = 2000;
        let mut seen = HashMap::new();
        let out = discover_from(
            &[old, new],
            &mut seen,
            |_| true,
            |_| Some("end_turn".into()),
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1, "同 cwd 合并为 1 个");
        assert_eq!(out[0].native_id, "61965", "primary = 最新 interactive");
        assert_eq!(
            out[0].status,
            AgentStatus::Done,
            "新会话 idle → Done,不被旧 busy+end_turn 污染"
        );
        assert!(seen.contains_key(&61965));
        assert!(!seen.contains_key(&72955), "旧 interactive 不进 seen");
    }

    #[test]
    fn discover_bg_still_merges_into_newest_interactive() {
        // 回归:bg 子进程的 busy 活跃度仍合并进最新 interactive 主(不破坏 fork 后台任务语义)。
        // 同 cwd:旧 interactive idle、新 interactive idle(=primary)、bg busy。
        let mut old = pf_cwd(72955, Some("idle"), Some("/a"));
        old.status_updated_at = 1000;
        let mut new = pf_cwd(61965, Some("idle"), Some("/a"));
        new.status_updated_at = 2000;
        let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
        bg.kind = Some("bg".into());
        bg.status_updated_at = 1500;
        let mut seen = HashMap::new();
        let out = discover_from(
            &[old, new, bg],
            &mut seen,
            |_| true,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].native_id, "61965", "primary = 最新 interactive");
        assert_eq!(
            out[0].status,
            AgentStatus::Working,
            "bg busy 合并进 primary → Working"
        );
        assert!(seen.contains_key(&61965));
        assert!(!seen.contains_key(&72955));
        assert!(!seen.contains_key(&100), "bg 不进 seen");
    }

    #[test]
    fn session_file_parses_camelcase_and_kind() {
        // 实测 session 文件是 camelCase + 含 kind/sessionId;rename_all 让 session_id 读到
        // (NeedsDeci 的 transcript 读取前提),kind 用以区分 interactive / bg 子 claude。
        let json = r#"{"pid":123,"sessionId":"abc","cwd":"/x","kind":"bg","status":"shell"}"#;
        let f: SessionFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.pid, 123);
        assert_eq!(f.session_id.as_deref(), Some("abc"));
        assert_eq!(f.cwd.as_deref(), Some("/x"));
        assert_eq!(f.kind.as_deref(), Some("bg"));
        assert_eq!(f.status.as_deref(), Some("shell"));
    }
}
