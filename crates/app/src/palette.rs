//! 灯的「颜色」映射。
//! 菜单栏灯 + 设置页色块用自绘彩色圆点(overlay::swatch_image,NSImage);浮窗用 NSColor
//! + CoreAnimation(见 overlay.rs)。下拉面板的会话列表用 emoji(palette::session_emoji)。

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

/// 下拉面板会话行的灯:Done-Notification 窗口期内(done_notif=true,浮窗/菜单栏全局灯
/// 为浅蓝时),Done 会话行同步显示 🔵,与全局灯颜色行为一致;窗口过期回退 🟢。非 Done
/// 会话不受影响(窗口期内全局态必为 Done,本就不会出现其他状态,此处仅防御)。
pub fn session_emoji(s: AgentStatus, done_notif: bool) -> &'static str {
    if done_notif && s == AgentStatus::Done {
        "🔵"
    } else {
        status_emoji(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_emoji_blue_only_in_done_notif_window() {
        // 窗口期内 Done 行变 🔵(与浮窗/菜单栏浅蓝一致),其余状态不受影响。
        assert_eq!(session_emoji(AgentStatus::Done, true), "🔵");
        assert_eq!(session_emoji(AgentStatus::Done, false), "🟢");
        assert_eq!(session_emoji(AgentStatus::Working, true), "🟡");
        assert_eq!(session_emoji(AgentStatus::Error, true), "🔴");
        assert_eq!(session_emoji(AgentStatus::Offline, true), "🟣");
        assert_eq!(session_emoji(AgentStatus::NeedsDeci, true), "🟠");
    }
}
