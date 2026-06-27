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

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
        discover_from(&files, &mut seen, Self::pid_alive, self.kind)
    }
}

/// 纯函数核心:给定本轮发现的文件集 + 历史可见状态 + 存活判定,决定每个会话的状态,
/// 并更新 `seen`。文件系统 / pid / 时间都被抽掉,便于 MOCK 单测。
///
/// 返回的会话按文件顺序;`seen` 会:记录本轮见到的 pid,并裁掉本轮没出现的(已被
/// Claude 清理 / 干净退出)。
fn discover_from(
    files: &[ParsedFile],
    seen: &mut HashMap<u32, AgentStatus>,
    is_alive: impl Fn(u32) -> bool,
    kind: AgentKind,
) -> Vec<AgentSession> {
    let mut live: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for f in files {
        let prev = seen.get(&f.pid).copied();
        let Some(st) = classify(f, prev, is_alive(f.pid)) else {
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
/// - pid 活:busy→Working,idle→Done,未知→Working;
/// - pid 死且 `seen` 里曾见过(活的)→ **Offline**(崩溃/被杀,文件残留);
/// - pid 死且从没见过 → `None`(古老残留,跳过,不制造噪音)。
fn classify(f: &ParsedFile, prev: Option<AgentStatus>, alive: bool) -> Option<AgentStatus> {
    if alive {
        Some(match f.status.as_deref() {
            Some("busy") => AgentStatus::Working,
            Some("idle") => AgentStatus::Done,
            _ => AgentStatus::Working, // 未知默认视为工作中
        })
    } else {
        prev.map(|_| AgentStatus::Offline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf(pid: u32, status: Option<&str>) -> ParsedFile {
        ParsedFile {
            pid,
            session_id: None,
            cwd: None,
            status: status.map(str::to_string),
        }
    }

    // ---- classify:纯函数 ----

    #[test]
    fn classify_alive_maps_status() {
        assert_eq!(
            classify(&pf(1, Some("busy")), None, true),
            Some(AgentStatus::Working)
        );
        assert_eq!(
            classify(&pf(1, Some("idle")), None, true),
            Some(AgentStatus::Done)
        );
        assert_eq!(
            classify(&pf(1, None), None, true),
            Some(AgentStatus::Working)
        );
        assert_eq!(
            classify(&pf(1, Some("wat")), None, true),
            Some(AgentStatus::Working)
        );
    }

    #[test]
    fn classify_dead_seen_before_is_offline() {
        // 曾见过(活的)→ 现在死了 = 失联
        assert_eq!(
            classify(&pf(1, Some("busy")), Some(AgentStatus::Working), false),
            Some(AgentStatus::Offline)
        );
        assert_eq!(
            classify(&pf(1, Some("idle")), Some(AgentStatus::Done), false),
            Some(AgentStatus::Offline)
        );
        // 上一轮就已经是 Offline,文件还残留 → 继续 Offline
        assert_eq!(
            classify(&pf(1, Some("busy")), Some(AgentStatus::Offline), false),
            Some(AgentStatus::Offline)
        );
    }

    #[test]
    fn classify_dead_never_seen_is_skipped() {
        // 古老残留 → None(不报)
        assert_eq!(classify(&pf(1, Some("busy")), None, false), None);
    }

    // ---- discover_from:MOCK(is_alive / files / seen 全注入)----

    #[test]
    fn discover_healthy_working() {
        let mut seen = HashMap::new();
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| true,
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(seen.get(&100), Some(&AgentStatus::Working));
    }

    #[test]
    fn discover_dead_seen_before_becomes_offline() {
        // 上一轮见过 100 在 Working;本轮 pid 死了 → Offline
        let mut seen = HashMap::from([(100, AgentStatus::Working)]);
        let out = discover_from(
            &[pf(100, Some("busy"))],
            &mut seen,
            |_| false,
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
            AgentKind::Claude,
        );
        assert!(out.is_empty());
        assert!(seen.is_empty());
    }

    #[test]
    fn discover_offline_recovers_to_working() {
        // 曾 Offline;进程复活且 busy → Working(sticky 机由上层解锁,这里只看 source 报 Working)
        let mut seen = HashMap::from([(300, AgentStatus::Offline)]);
        let out = discover_from(
            &[pf(300, Some("busy"))],
            &mut seen,
            |_| true,
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
            AgentKind::Claude,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].status, AgentStatus::Working);
        assert_eq!(out[1].status, AgentStatus::Offline);
    }
}
