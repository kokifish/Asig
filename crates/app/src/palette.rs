//! 灯的「颜色」映射。
//! Phase-1 MVP:菜单栏灯用 emoji(零图片资源,确定能编译/运行)。
//! Phase-2:浮窗用 NSColor + CoreAnimation(见 overlay.rs)。

use agent_light_core::{AgentStatus, Color};

/// 按颜色取 emoji(菜单栏灯用)。深绿无现成圆点 emoji,用心形 💚 近似(Done Notification)。
pub fn color_emoji(c: Color) -> &'static str {
    match c {
        Color::Green => "🟢",
        Color::DarkGreen => "💚",
        Color::Yellow => "🟡",
        Color::Amber => "🟠",
        Color::Red => "🔴",
        Color::Purple => "🟣",
    }
}

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
pub fn anim_name(a: agent_light_core::Anim) -> &'static str {
    use agent_light_core::Anim;
    match a {
        Anim::Steady => "常亮",
        Anim::Pulse => "呼吸",
        Anim::Blink => "明灭",
        Anim::Ripple => "波纹",
    }
}
