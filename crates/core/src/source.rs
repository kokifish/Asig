//! Source 层:每个 agent 工具实现一个 AgentSource。UI 无关、可移植。

use crate::status::AgentStatus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 被监控的 agent 种类。serde 序列化(存 `Settings.enabled_agents`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    CodeBuddy, // 暂不支持(实现保留,见 claude.rs);保留 variant 保 serde 向后兼容。
    OpenClaw,
    Hermes,
    Trae, // 暂未实现;Accessibility 路线见 README 长期目标。
}

impl AgentKind {
    /// 全部已支持的 agent(chip 顺序 = 默认启用顺序)。CodeBuddy 暂不支持、Trae 暂未实现,均不含。
    /// `config::default_enabled_agents` / `Monitor::default` / 设置 chip 共用此单一事实源。
    pub const IMPLEMENTED: [Self; 3] = [Self::Claude, Self::OpenClaw, Self::Hermes];

    /// 用户可见的全称(下拉会话列表等展示用)。变体名是简写,展示用全称(Claude → Claude Code)。
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::CodeBuddy => "CodeBuddy",
            Self::OpenClaw => "OpenClaw",
            Self::Hermes => "Hermes",
            Self::Trae => "Trae",
        }
    }
}

/// 一个被发现的 agent 会话(状态已由 source 内部解析归一)。
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub kind: AgentKind,
    /// 跨工具唯一键:`{kind}:{native_id}`。
    pub id: String,
    pub native_id: String,
    pub cwd: Option<PathBuf>,
    pub status: AgentStatus,
    pub label: Option<String>,
    /// 最近一条 user 文本消息(已规范化),Panel「start 事件」展示用。None = 取不到。
    pub last_user_msg: Option<String>,
    /// 最近一条 assistant 文本回复(已规范化),Panel「done 事件」展示用。None = 取不到。
    pub last_assistant_msg: Option<String>,
}

impl AgentSession {
    /// 会话可读标识(Panel 会话列表 + 事件列表的单一事实源):OpenClaw/Hermes 显示
    /// agent 名(main/kotomi/…);Claude 显示 cwd basename(比 session UUID 易读)。
    pub fn display_label(&self) -> String {
        match self.kind {
            AgentKind::OpenClaw | AgentKind::Hermes => {
                self.label.clone().unwrap_or_else(|| "-".into())
            }
            _ => self
                .cwd
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("-")
                .to_string(),
        }
    }
}

/// 每个工具实现一个。
///
/// - **poll 路径**:`discover()` 立即扫描并返回(当前实现)。
/// - **push 路径**(hook / 文件监听,未来路线):降低延迟、拿到精准的
///   needs-decision / error。届时扩展本 trait(见 README),核心循环不变。
pub trait AgentSource: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn discover(&self) -> Vec<AgentSession>;
}
