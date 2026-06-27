//! 灯的「颜色」映射。
//! 菜单栏灯 + 设置页色块用自绘彩色圆点(overlay::swatch_image,NSImage);浮窗用 NSColor
//! + CoreAnimation(见 overlay.rs)。下拉面板的会话列表用 emoji(palette::status_emoji)。

use agent_light_core::AgentStatus;

pub fn status_emoji(s: AgentStatus) -> &'static str {
    match s {
        AgentStatus::Working => "🟡",
        AgentStatus::NeedsDeci => "🟠",
        AgentStatus::Done => "🟢",
        AgentStatus::Error => "🔴",
        AgentStatus::Offline => "🟣",
    }
}
