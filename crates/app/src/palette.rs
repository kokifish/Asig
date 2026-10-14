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

/// 下拉面板会话行的灯:**该会话自身**刚转入 Done(done-notif 窗口内,`Snapshot::done_notif_ids`)
/// → 🔵,与全局灯此刻的浅蓝呼应;闲置已久的 Done 行保持 🟢,不随别的会话的完成通知变蓝。
/// 窗口过期回退 🟢;非 Done 会话不受影响。
pub fn session_emoji(s: AgentStatus, just_done: bool) -> &'static str {
    if just_done && s == AgentStatus::Done {
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
        // 该会话自身刚转入 Done → 🔵;其余(含窗口内但非刚完成的 Done)按状态色。
        assert_eq!(session_emoji(AgentStatus::Done, true), "🔵");
        assert_eq!(session_emoji(AgentStatus::Done, false), "🟢");
        assert_eq!(session_emoji(AgentStatus::Working, true), "🟡");
        assert_eq!(session_emoji(AgentStatus::Error, true), "🔴");
        assert_eq!(session_emoji(AgentStatus::Offline, true), "🟣");
        assert_eq!(session_emoji(AgentStatus::NeedsDeci, true), "🟠");
    }
}
