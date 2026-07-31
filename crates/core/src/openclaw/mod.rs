//! OpenClaw 状态出口:只读主状态库 + 各 agent 交互式会话 jsonl,按 agent 聚合映射状态。
//!
//! 两套数据源:
//! 1. **后台任务** — `~/.openclaw/state/openclaw.sqlite`(WAL,单一事实源、`.migrated`
//!    合并目标,升级只朝它收敛):只读打开 → 查询 → 丢弃;WAL 允许 N 读 + 1 写并发,
//!    不抢 openclaw 写锁;连接局部、源结构只存 root,天然 Send+Sync。
//! 2. **交互式会话** — `agents/<id>/sessions/<sid>.jsonl`(TUI/webchat 事件流,**不进主库**):
//!    读尾部最后一条 message 的 `role` + `message.stopReason` 判在跑(类比 Claude 的
//!    `stop_reason`)—— 不依赖写入连续性,工具执行的长间隙不会误判完成。
//!
//! 状态映射(每个近期 agent 一个会话):
//!   - 后台:task_runs/flow_runs/subagent_runs 跨表归并(`ended_at IS NULL`→Working);
//!   - 交互式:尾部 `role∈{user,toolResult}` 或 `stop_reason='toolUse'` → Working;
//!   - `flow_runs.status='blocked'` 且 `ended_at IS NULL` → NeedsDeci(已结束的 blocked
//!     投递失败终态不计);
//!   - 近期(`ERROR_RECENT_MS` 内)failed/lost/subagent-error、或交互式尾部近期 `stopReason='error'` → Error;否则 Done。
//!
//! 每 agent 取一个观测态(Error > NeedsDeci > Working > Done);Error 过窗后报 Done,
//! 由 `lib.rs` 的 sticky `transition()` 自动解锁。

mod db;
mod probe;
mod sessions;

pub use probe::{AgentProbe, probe};

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use db::{AgentAcc, collect};
use rusqlite::Connection;
use sessions::{SessionSignal, latest_session_signals};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct OpenClawSource {
    root: PathBuf,
}

impl OpenClawSource {
    pub fn new() -> Option<Self> {
        Some(Self {
            root: dirs::home_dir()?.join(".openclaw"),
        })
    }

    /// 主库路径(cli 打印 / 单一事实源)。
    pub fn db_path(&self) -> PathBuf {
        self.root.join("state").join("openclaw.sqlite")
    }

    /// 会话 jsonl 根目录(discover/probe 扫 agents/<id>/sessions 用)。
    pub(super) fn root_path(&self) -> &Path {
        &self.root
    }

    /// 只读打开主库(WAL,不抢 openclaw 写锁)。失败 → None(discover/probe 据此回退)。
    fn connect(&self) -> Option<Connection> {
        crate::sys::open_readonly(&self.db_path())
    }
}

impl AgentSource for OpenClawSource {
    fn kind(&self) -> AgentKind {
        AgentKind::OpenClaw
    }

    fn discover(&self) -> Vec<AgentSession> {
        let Some(conn) = self.connect() else {
            // 打不开:没装 openclaw(静默)vs 库损坏(应可见)。提示路径便于排障。
            log::warn!("openclaw 库打不开: {}", self.db_path().display());
            return Vec::new();
        };
        let signals = latest_session_signals(&self.root);
        discover_from(&conn, crate::sys::now_ms(), &signals)
    }
}

/// 查询 + 归并核心(纯函数:接连接 + 当前 ms + 各 agent 交互式会话尾部信号)。
fn discover_from(
    conn: &Connection,
    now: u64,
    session_signals: &HashMap<String, SessionSignal>,
) -> Vec<AgentSession> {
    collect(conn, now, session_signals)
        .into_iter()
        .map(|(aid, acc, sig)| AgentSession {
            kind: AgentKind::OpenClaw,
            id: format!("OpenClaw:{aid}"),
            native_id: aid.clone(),
            cwd: None,
            status: classify_agent(acc),
            label: Some(aid),
            last_user_msg: sig.as_ref().and_then(|s| s.last_user_msg.clone()),
            last_assistant_msg: sig.as_ref().and_then(|s| s.last_assistant_msg.clone()),
        })
        .collect()
}

/// 纯函数:单 agent 观测态(优先级 Error > NeedsDeci > Working > Done)。
/// Error 同时覆盖「近期 failed」(recent_err)与「协调态卡死」(stuck)。
fn classify_agent(a: AgentAcc) -> AgentStatus {
    if a.recent_err || a.stuck {
        AgentStatus::Error
    } else if a.blocked {
        AgentStatus::NeedsDeci
    } else if a.running || a.run_active {
        AgentStatus::Working
    } else {
        AgentStatus::Done
    }
}

#[cfg(test)]
mod tests;
