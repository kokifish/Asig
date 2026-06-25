//! 灯的「颜色」映射。
//! Phase-1 MVP:菜单栏灯用 emoji(零图片资源,确定能编译/运行)。
//! Phase-2:浮窗用 NSColor + CoreAnimation(见 overlay.rs)。

use agent_light_core::{AgentStatus, Color};

pub fn status_emoji(s: AgentStatus) -> &'static str {
    match s {
        AgentStatus::Working => "🟡",
        AgentStatus::NeedsDeci => "🟠",
        AgentStatus::Done => "🟢",
        AgentStatus::Error => "🔴",
        AgentStatus::Offline => "🟣",
    }
}

#[allow(dead_code)]
pub fn color_name(c: Color) -> &'static str {
    match c {
        Color::Green => "green",
        Color::DarkGreen => "dark_green",
        Color::Yellow => "yellow",
        Color::Amber => "amber",
        Color::Red => "red",
        Color::Purple => "purple",
    }
}
