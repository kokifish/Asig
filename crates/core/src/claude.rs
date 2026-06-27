//! Claude Code 与 CodeBuddy 共用同一实现。
//!
//! 两者都是"会话状态文件 + (可选)hook"模式,且文件格式同构:
//!   ~/.claude/sessions/<pid>.json    status: "busy" | "idle"
//!   ~/.codebuddy/sessions/<pid>.json (CodeBuddy 是 Claude Code hook 的兼容 clone)
//! 区别仅在根目录与进程名 —— 所以一个 ClaudeLikeSource 参数化复用。
//!
//! Offline 检测(廉价、可靠):
//!   - `status` 字段只有 busy/idle,没有 error/offline;
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

/// ~/.claude|~/.codebuddy/sessions/<pid>.json 的结构(实测,版本 2.1.x)。
#[derive(Deserialize)]
struct SessionFile {
    pid: u32,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    status: Option<String>, // "busy" | "idle"
}

/// 与文件解析同构的纯数据(供纯函数 `discover_from` / 单测用,无需 serde)。
#[derive(Debug, Clone)]
pub(crate) struct ParsedFile {
    pub pid: u32,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<String>,
}

impl From<&SessionFile> for ParsedFile {
    fn from(f: &SessionFile) -> Self {
        Self {
            pid: f.pid,
            session_id: f.session_id.clone(),
            cwd: f.cwd.clone(),
            status: f.status.clone(),
        }
    }
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
        unsafe { libc::kill(pid as i32, 0) == 0 }
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
            files.push(ParsedFile::from(&f));
        }
        let mut seen = self.seen.lock().unwrap();
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
/// 返回的会话按文件顺序;`seen` 会:记录本轮见到的 pid,并裁掉本轮没出现的(已被
/// Claude 清理 / 干净退出)。
fn discover_from(
    files: &[ParsedFile],
    seen: &mut HashMap<u32, AgentStatus>,
    is_alive: impl Fn(u32) -> bool,
    stop_reason_of: impl Fn(&str) -> Option<String>,
    kind: AgentKind,
) -> Vec<AgentSession> {
    let mut live: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for f in files {
        let prev = seen.get(&f.pid).copied();
        let alive = is_alive(f.pid);
        // 只对 busy 会话读 transcript(idle→Done 无需、省一次文件读)。
        let sr = if alive && f.status.as_deref() == Some("busy") {
            f.session_id.as_deref().and_then(&stop_reason_of)
        } else {
            None
        };
        let Some(st) = classify(f, prev, alive, sr.as_deref()) else {
            continue;
        };
        seen.insert(f.pid, st);
        live.insert(f.pid);
        out.push(AgentSession {
            kind,
            id: format!("{:?}:{}", kind, f.pid),
            native_id: f.pid.to_string(),
            cwd: f.cwd.clone().map(PathBuf::from),
            project: None,
            status: st,
            label: f.session_id.clone(),
        });
    }
    // 本轮没出现的 pid(文件消失)→ 不再盯。干净退出就这样被自然忘掉。
    seen.retain(|pid, _| live.contains(pid));
    out
}

/// 单文件状态判定(纯函数)。
///
/// - pid 活且 `idle` → Done;
/// - pid 活且 `busy`:`stop_reason == "end_turn"`(模型说完、等用户回)→ NeedsDeci;
///   `tool_use`/未知/读不到 → Working(正在跑工具);
/// - pid 活、status 未知 → Working;
/// - pid 死且 `seen` 里曾见过(活的)→ **Offline**(崩溃/被杀,文件残留);
/// - pid 死且从没见过 → `None`(古老残留,跳过,不制造噪音)。
fn classify(
    f: &ParsedFile,
    prev: Option<AgentStatus>,
    alive: bool,
    stop_reason: Option<&str>,
) -> Option<AgentStatus> {
    if alive {
        Some(match f.status.as_deref() {
            Some("idle") => AgentStatus::Done,
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

/// 只读文件尾部 ~16KB(避免对大 transcript 全量读),反序找最后一条带 `stop_reason` 的行。
/// 用 lossy 解码(尾部起点可能落在多字节字符中间),首行多半被截断故丢弃。
fn read_tail_stop_reason(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    const TAIL: u64 = 16_384;
    let start = size.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 {
        lines.remove(0); // 起点非文件首 → 首行多半被截断,丢弃
    }
    for line in lines.iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
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

    fn pf(pid: u32, status: Option<&str>) -> ParsedFile {
        ParsedFile {
            pid,
            session_id: Some(format!("s{pid}")), // 有 session_id 才会触发 transcript 读取
            cwd: None,
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
        // 100 活着 busy;200 上轮见过、现在死了 → 一个 Working 一个 Offline
        let mut seen = HashMap::from([(200, AgentStatus::Working)]);
        let out = discover_from(
            &[pf(100, Some("busy")), pf(200, Some("busy"))],
            &mut seen,
            |pid| pid == 100,
            |_| None,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(out[1].status, AgentStatus::Offline);
    }
}
