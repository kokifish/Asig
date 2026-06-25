//! 聚合:把 N 个会话压成灯态(每工具一颗 + 一个全局)。

use crate::source::{AgentKind, AgentSession};
use crate::status::AgentStatus;
use std::collections::HashMap;

/// 全局灯 = 所有会话里最高优先级的状态;无会话则 Offline。
pub fn global_status(sessions: &[AgentSession]) -> AgentStatus {
    sessions
        .iter()
        .map(|s| s.status)
        .max_by_key(|s| s.priority())
        .unwrap_or(AgentStatus::Offline)
}

/// 每个工具的聚合灯态(同工具多会话取最严重者)。
pub fn per_kind(sessions: &[AgentSession]) -> HashMap<AgentKind, AgentStatus> {
    let mut best: HashMap<AgentKind, AgentStatus> = HashMap::new();
    for s in sessions {
        let cur = best.get(&s.kind).copied().unwrap_or(AgentStatus::Offline);
        if s.status.priority() > cur.priority() {
            best.insert(s.kind, s.status);
        }
    }
    best
}
