//! Claude Code 与 CodeBuddy 共用同一实现。
//!
//! 两者都是"会话状态文件 + (可选)hook"模式,且文件格式同构:
//!   ~/.claude/sessions/<pid>.json    status: "busy" | "idle" | "shell"
//!   ~/.codebuddy/sessions/<pid>.json (CodeBuddy 是 Claude Code hook 的兼容 clone)
//! 区别仅在根目录与进程名 —— 所以一个 ClaudeLikeSource 参数化复用。
//!
//! **按 cwd 聚合**:同目录下的多个 session(用户手开的 interactive + claude
//! `--fork-session` 派发的后台子 claude `kind:"bg"`)合并为**一个**会话 —— interactive
//! 作主,bg 不单独显示,但其 busy 活跃度合并进主会话状态。否则把任务 fork 到后台跑时,
//! 主进程会 idle 成 shell,Asig 会误判为不在运行。纯 bg 无 interactive 的目录跳过(避免
//! 与 OpenClaw source 重叠)。
//!
//! Offline 检测(廉价、可靠):
//!   - `status` 字段只有 busy/idle/shell,没有 error/offline;
//!   - `statusUpdatedAt` 实测只在**状态转换**时写,不是周期心跳(busy 会话跑很久也
//!     不更新),故**不能**用心跳新鲜度判"卡死"——会误报长任务;
//!   - 可靠信号:进程死了。Claude 干净退出会清掉 session 文件;**残留的死 pid 文件
//!     = 崩溃/被杀**。Asig 只对"本轮之前见过它活着"的会话报 Offline,过滤掉古老残留。
//!
//! NeedsDeci(待决策)检测:
//!   - session 文件的 `status` 在"Claude 问你问题"时**仍是 busy**(turn 还没结束),
//!     故单看 status 只能区分 busy(Working)/idle(Done),永远到不了 NeedsDeci。
//!   - 真正信号在会话 transcript(`~/.claude/projects/*/<sessionId>.jsonl`)的最后一条
//!     `stop_reason`:busy 且 `end_turn`(模型说完、把控制权交还用户)→ NeedsDeci(等你
//!     输入/决策);`tool_use`/未知 → Working(正在跑工具)。只读文件尾部 ~16KB,3s 一次
//!     轮询开销可忽略;读不到 transcript → 回退 Working(不报错)。

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// ~/.claude|~/.codebuddy/sessions/<pid>.json 的结构(实测,版本 2.1.x;字段 camelCase)。
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
    /// 整组跳过(避免与 OpenClaw source 重叠)。CodeBuddy 等无此字段则为 None(当 interactive)。
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>, // "busy" | "idle" | "shell"
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

    pub fn codebuddy() -> Option<Self> {
        Some(Self {
            kind: AgentKind::CodeBuddy,
            root: dirs::home_dir()?.join(".codebuddy"),
            seen: Mutex::new(HashMap::new()),
        })
    }

    /// kill(pid, 0) == 0 表示进程存活;ESRCH(不在)返回非 0。
    fn pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 不发信号,只探测进程存在性;pid 来自本地 session 文件,非敌对输入。
        let pid = i32::try_from(pid).unwrap_or(-1);
        pid >= 0 && unsafe { libc::kill(pid, 0) == 0 }
    }
}

impl AgentSource for ClaudeLikeSource {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    fn discover(&self) -> Vec<AgentSession> {
        let dir = self.root.join("sessions");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new(); // 没装该工具 → 空目录 → 无会话
        };
        let mut files = Vec::new();
        for e in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(f): Result<SessionFile, _> = serde_json::from_str(&text) else {
                continue;
            };
            files.push(f);
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let root = &self.root;
        discover_from(
            &files,
            &mut seen,
            Self::pid_alive,
            |sid| last_stop_reason(root, sid),
            self.kind,
        )
    }
}

/// 纯函数核心:给定本轮发现的文件集 + 历史可见状态 + 存活判定 + stop_reason 探测,决定
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
    stop_reason_of: impl Fn(&str) -> Option<String>,
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
        // 主 = 组内首个 interactive(kind != bg);纯 bg 无主 → 跳过整组(避免与 OpenClaw 重叠)。
        let Some(primary) = group
            .iter()
            .find(|f| f.kind.as_deref() != Some("bg"))
            .copied()
        else {
            continue;
        };
        let prev = seen.get(&primary.pid).copied();
        let Some(st) = group_status(primary, group, prev, &is_alive, &stop_reason_of) else {
            continue;
        };
        seen.insert(primary.pid, st);
        live.insert(primary.pid);
        out.push(AgentSession {
            kind,
            id: format!("{:?}:{}", kind, primary.pid),
            native_id: primary.pid.to_string(),
            cwd: primary.cwd.clone().map(PathBuf::from),
            project: None,
            status: st,
            label: primary.session_id.clone(),
        });
    }
    // 本轮没出现的(主)pid → 不再盯。干净退出就这样被自然忘掉。
    seen.retain(|pid, _| live.contains(pid));
    out
}

/// 组内(同 cwd 的 interactive 主 + bg 子进程)聚合状态:对每个 file 调 `classify`,
/// 取**最活跃**的(NeedsDeci > Working > Done > Offline)。bg 子进程的 busy 据此贡献给主会话。
fn group_status(
    primary: &SessionFile,
    group: &[&SessionFile],
    prev_of_primary: Option<AgentStatus>,
    is_alive: &impl Fn(u32) -> bool,
    stop_reason_of: &impl Fn(&str) -> Option<String>,
) -> Option<AgentStatus> {
    let mut best: Option<AgentStatus> = None;
    for &f in group {
        let prev = if f.pid == primary.pid {
            prev_of_primary
        } else {
            None
        };
        let alive = is_alive(f.pid);
        // 只对 busy 进程读 transcript(idle/shell→Done 无需、省一次文件读)。
        let sr = if alive && f.status.as_deref() == Some("busy") {
            f.session_id.as_deref().and_then(stop_reason_of)
        } else {
            None
        };
        let Some(st) = classify(f, prev, alive, sr.as_deref()) else {
            continue;
        };
        best = Some(most_active(best, st));
    }
    best
}

/// 活跃度排序:NeedsDeci > Working > Done > Offline;返回更活跃者。
fn most_active(a: Option<AgentStatus>, b: AgentStatus) -> AgentStatus {
    fn rank(st: AgentStatus) -> u8 {
        match st {
            AgentStatus::NeedsDeci => 4,
            AgentStatus::Error => 4, // 出错也需关注;Claude source 不产生(OpenClaw 才有)
            AgentStatus::Working => 3,
            AgentStatus::Done => 2,
            AgentStatus::Offline => 1,
        }
    }
    match a {
        Some(prev) if rank(prev) >= rank(b) => prev,
        _ => b,
    }
}

/// 单文件状态判定(纯函数)。
///
/// - pid 活且 `idle`/`shell` → Done;
/// - pid 活且 `busy`:`stop_reason == "end_turn"`(模型说完、等用户回)→ NeedsDeci;
///   `tool_use`/未知/读不到 → Working(正在跑工具);
/// - pid 活、status 未知 → Working;
/// - pid 死且 `seen` 里曾见过(活的)→ **Offline**(崩溃/被杀,文件残留);
/// - pid 死且从没见过 → `None`(古老残留,跳过,不制造噪音)。
fn classify(
    f: &SessionFile,
    prev: Option<AgentStatus>,
    alive: bool,
    stop_reason: Option<&str>,
) -> Option<AgentStatus> {
    if alive {
        Some(match f.status.as_deref() {
            // idle/shell = 空闲(shell=Claude REPL 等输入,无活跃任务)→ Done,非 Working。
            Some("idle") | Some("shell") => AgentStatus::Done,
            Some("busy") => match stop_reason {
                Some("end_turn") => AgentStatus::NeedsDeci,
                _ => AgentStatus::Working, // tool_use / 未知 / 读不到 → 正在跑
            },
            _ => AgentStatus::Working,
        })
    } else {
        prev.map(|_| AgentStatus::Offline)
    }
}

/// 读会话 transcript(`<root>/projects/*/<sessionId>.jsonl`)尾部最后一条 `stop_reason`。
/// busy 会话据此区分 NeedsDeci(end_turn)vs Working(tool_use)。读不到 → None(回退 Working)。
fn last_stop_reason(root: &Path, session_id: &str) -> Option<String> {
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return None;
    };
    for e in entries.flatten() {
        let p = e.path().join(format!("{session_id}.jsonl"));
        if p.is_file() {
            return read_tail_stop_reason(&p);
        }
    }
    None
}

/// 只读文件尾部 ~16KB,反序找最后一条带 `message.stop_reason` 的行。
/// 尾部 I/O(seek+read+lossy+丢首行+解析)走共用 `jsonl_tail::read_tail_lines`。
fn read_tail_stop_reason(path: &Path) -> Option<String> {
    let events = crate::jsonl_tail::read_tail_lines(path, 16_384)?;
    for v in events.iter().rev() {
        if let Some(sr) = v
            .get("message")
            .and_then(|m| m.get("stop_reason"))
            .and_then(|s| s.as_str())
        {
            return Some(sr.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
