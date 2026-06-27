//! OpenClaw 状态出口:SQLite 状态库(本机已验证)。
//!
//!   ~/.openclaw/state/openclaw.sqlite  (WAL 模式,实时性好)
//!   task_runs.status: succeeded | failed ; ended_at NULL => 运行中
//!   subagent_runs / flow_runs(blocked_task_id) / acp_sessions.state
//!   细粒度实时流:~/.openclaw/internal-agent-runs/<runId>.trajectory.jsonl
//!
//! Phase 3 启用 rusqlite 后补全;当前只做目录存在性判断。

use crate::source::{AgentKind, AgentSession, AgentSource};
use std::path::PathBuf;

pub struct OpenClawSource {
    root: PathBuf,
}

impl OpenClawSource {
    pub fn new() -> Option<Self> {
        Some(Self {
            root: dirs::home_dir()?.join(".openclaw"),
        })
    }
}

impl AgentSource for OpenClawSource {
    fn kind(&self) -> AgentKind {
        AgentKind::OpenClaw
    }

    fn discover(&self) -> Vec<AgentSession> {
        // TODO(Phase 3): 打开 state/openclaw.sqlite,查询:
        //   SELECT task_id, status, error, ended_at
        //   FROM task_runs
        //   WHERE ended_at IS NULL OR ended_at > <recent>;
        //   映射: ended_at NULL -> Working
        //         status='succeeded' -> Done
        //         status='failed' / error!='' -> Error
        //   needs-decision: 查 flow_runs.blocked_task_id 非空,或
        //                    tail internal-agent-runs/*.trajectory.jsonl 找等待事件。
        if !self.root.join("state/openclaw.sqlite").exists() {
            return Vec::new();
        }
        Vec::new()
    }
}
