//! 灯的「颜色」映射。
//! 菜单栏灯用自绘彩色圆点(tray.rs::circle_image,NSImage);浮窗用 NSColor + CoreAnimation(见 overlay.rs)。
//! 下拉面板的会话列表用 emoji(palette::status_emoji)。

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

/// 颜色的中文名(设置面板颜色下拉用)。
pub fn color_name(c: Color) -> &'static str {
    match c {
        Color::Green => "绿",
        Color::DarkGreen => "深绿",
        Color::Yellow => "黄",
        Color::Amber => "琥珀",
        Color::Red => "红",
        Color::Purple => "紫",
    }
}

/// 动画的中文名(设置面板动画下拉用)。
/// 快闪 / 慢闪 / 呼吸都是 Pulse(只是周期不同),故下拉里只有 3 个动效。
pub fn anim_name(a: agent_light_core::Anim) -> &'static str {
    use agent_light_core::Anim;
    match a {
        Anim::Steady => "常亮",
        Anim::Pulse => "呼吸",
        Anim::Ripple => "波纹",
    }
}
