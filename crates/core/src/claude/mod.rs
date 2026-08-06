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
    /// session_id → transcript 路径缓存。避免每轮对 busy 会话全扫 `projects/`(其子目录随
    /// 历史无界增长)。session_id 唯一不复用 → 缓存只增,增量可控(每项 ~80B)。
    transcript_paths: Mutex<HashMap<String, Option<PathBuf>>>,
}

impl ClaudeLikeSource {
    pub fn claude() -> Option<Self> {
        Some(Self {
            kind: AgentKind::Claude,
            root: dirs::home_dir()?.join(".claude"),
            seen: Mutex::new(HashMap::new()),
            transcript_paths: Mutex::new(HashMap::new()),
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
        let mut seen = self.seen.lock().unwrap_or_else(|e| {
            // 锁中毒(持有者 panic,本 source 几乎不会):重置而非沿用脏 map,
            // 避免后续轮询在中毒的可见状态上叠加错误。
            log::warn!("claude seen 锁中毒,重置");
            let mut g = e.into_inner();
            g.clear();
            g
        });
        let root = &self.root;
        let cache = &self.transcript_paths;
        // 带 path 缓存的 transcript 探测:首次未命中扫一次 projects/,后续轮询直接读缓存路径,
        // 把 N 个 busy 会话 × 全扫 projects/ 降到 N 次 O(1) 查表 + 至多一次扫描。
        let signal_of = |sid: &str| -> Option<TailInfo> {
            let path = {
                let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
                c.get(sid).cloned().unwrap_or_else(|| {
                    let found = scan_transcript(root, sid);
                    c.insert(sid.to_string(), found.clone());
                    found
                })
            };
            path.as_deref().and_then(read_tail)
        };
        discover_from(
            &files,
            &mut seen,
            crate::sys::pid_alive,
            signal_of,
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
    signal_of: impl Fn(&str) -> Option<TailInfo>,
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
        let Some((st, tail)) = group_status(primary, group, prev, &is_alive, &signal_of) else {
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
            last_user_msg: tail.as_ref().and_then(|t| t.last_user_msg.clone()),
            last_assistant_msg: tail.as_ref().and_then(|t| t.last_assistant_msg.clone()),
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
    signal_of: &impl Fn(&str) -> Option<TailInfo>,
) -> Option<(AgentStatus, Option<TailInfo>)> {
    let mut best: Option<AgentStatus> = None;
    let mut primary_tail: Option<TailInfo> = None;
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
        let is_primary = f.pid == primary.pid;
        // primary 总读 transcript:idle/shell(Done)时也要取 last_assistant_msg 给 Done 事件
        // (其 signal 在 idle 时被 classify 忽略,不影响 status)。bg 仅 busy 读(只为活跃度)。
        let tail = if alive && (is_primary || f.status.as_deref() == Some("busy")) {
            f.session_id.as_deref().and_then(signal_of)
        } else {
            None
        };
        if is_primary {
            primary_tail = tail.clone();
        }
        let Some(st) = classify(f, prev, alive, tail.as_ref().map(|t| t.signal.as_str())) else {
            continue;
        };
        best = Some(most_active(best, st));
    }
    best.map(|st| (st, primary_tail))
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

/// 在 `<root>/projects/*/` 下找 `<session_id>.jsonl`(线性扫;首次未命中后由调用方缓存)。
fn scan_transcript(root: &Path, session_id: &str) -> Option<PathBuf> {
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return None;
    };
    for e in entries.flatten() {
        let p = e.path().join(format!("{session_id}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 读会话 transcript 尾部信号(扫描 + 读尾,无缓存 —— probe 用;discover 用带缓存的闭包)。
fn last_signal(root: &Path, session_id: &str) -> Option<TailInfo> {
    scan_transcript(root, session_id)
        .as_deref()
        .and_then(read_tail)
}

/// transcript 尾部提取结果:`signal`(状态判定)+ 最近 user/assistant 文本(Panel 事件用)。
/// 一次尾部 I/O 同时算出,避免重复读文件。
#[derive(Clone)]
struct TailInfo {
    signal: String,
    last_user_msg: Option<String>,
    last_assistant_msg: Option<String>,
}

/// 只读文件尾部 ~16KB,反序遍历:`signal` 取最后一条有意义事件(`type:"user"` → `"user"`;
/// `type:"assistant"` → 其 `message.stop_reason`),并顺带提取最近 user 文本 + 最近有 text
/// 的 assistant 文本(`message.content`,string 或 `[{type:"text",text:"…"}]` 数组,跳过纯
/// tool_use)。这样 end_turn 之后若已有 user 消息,判 Working 而非残留 end_turn 误判
/// NeedsDeci。读不到 / 无有意义事件 → None(上游回退 Working)。
fn read_tail(path: &Path) -> Option<TailInfo> {
    let events = crate::jsonl_tail::read_tail_lines(path, 16_384)?;
    let mut signal: Option<String> = None;
    let mut last_user_msg: Option<String> = None;
    let mut last_assistant_msg: Option<String> = None;
    for v in events.iter().rev() {
        let ty = v.get("type").and_then(|t| t.as_str());
        if signal.is_none() {
            if ty == Some("user") {
                signal = Some("user".to_string());
            } else if ty == Some("assistant") {
                if let Some(sr) = v
                    .get("message")
                    .and_then(|m| m.get("stop_reason"))
                    .and_then(|s| s.as_str())
                {
                    signal = Some(sr.to_string());
                }
            }
        }
        let content = v.get("message").and_then(|m| m.get("content"));
        if last_user_msg.is_none() && ty == Some("user") {
            last_user_msg = crate::jsonl_tail::extract_text(content);
        }
        if last_assistant_msg.is_none() && ty == Some("assistant") {
            if let Some(t) = crate::jsonl_tail::extract_text(content) {
                last_assistant_msg = Some(t);
            }
        }
        if signal.is_some() && last_user_msg.is_some() && last_assistant_msg.is_some() {
            break;
        }
    }
    Some(TailInfo {
        signal: signal?,
        last_user_msg,
        last_assistant_msg,
    })
}

// ---- CLI 探针(`probe-claude`)与测试拆到子模块(主逻辑 < 380 行,对齐 openclaw/hermes 结构)----
pub mod probe;
pub use probe::probe;

#[cfg(test)]
mod tests;
