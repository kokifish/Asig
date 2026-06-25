//! Source 层:每个 agent 工具实现一个 AgentSource。UI 无关、可移植。

use crate::status::AgentStatus;
use std::path::PathBuf;

/// 被监控的 agent 种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    CodeBuddy,
    OpenClaw,
    Trae, // 暂未实现;Accessibility 路线见 README 长期目标。
}

/// 一个被发现的 agent 会话(状态已由 source 内部解析归一)。
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub kind: AgentKind,
    /// 跨工具唯一键:`{kind}:{native_id}`。
    pub id: String,
    pub native_id: String,
    pub cwd: Option<PathBuf>,
    pub project: Option<String>,
    pub status: AgentStatus,
    pub label: Option<String>,
}

/// 每个工具实现一个。
///
/// - **poll 路径**:`discover()` 立即扫描并返回(当前实现)。
/// - **push 路径**(hook / 文件监听,Phase 2/3):降低延迟、拿到精准的
///   needs-decision / error。届时扩展本 trait(见 README),核心循环不变。
pub trait AgentSource: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn discover(&self) -> Vec<AgentSession>;
}
