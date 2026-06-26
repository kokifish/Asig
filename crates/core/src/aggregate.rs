//! 聚合:把 N 个会话压成全局灯态。

use crate::source::AgentSession;
use crate::status::AgentStatus;

/// 全局灯 = 所有会话里最高优先级的状态;无会话则 Offline。
pub fn global_status(sessions: &[AgentSession]) -> AgentStatus {
    sessions
        .iter()
        .map(|s| s.status)
        .max_by_key(|s| s.priority())
        .unwrap_or(AgentStatus::Offline)
}
