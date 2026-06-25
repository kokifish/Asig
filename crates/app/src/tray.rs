//! 菜单栏灯:NSStatusItem + 按钮 emoji。点击按钮弹 popover(见 panel.rs)。

use agent_light_core::AgentStatus;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject};
use objc2::{class, msg_send, msg_send_id, sel, DeclaredClass};
use objc2_app_kit::{NSStatusBar, NSStatusBarButton, NSStatusItem};
use objc2_foundation::{NSString, NSTimer};

use crate::app_delegate::AppDelegate;
use crate::palette::status_emoji;

/// 建状态栏项,并把按钮点击接到 `togglePopover:`。
pub fn build(delegate: &Retained<AppDelegate>) {
    let sb: Retained<NSStatusBar> =
        unsafe { msg_send_id![class!(NSStatusBar), systemStatusBar] };
    let item: Retained<NSStatusItem> =
        unsafe { msg_send_id![&sb, statusItemWithLength: -1.0f64] };
    set_light(&item, AgentStatus::Offline, false);

    // 点状态栏按钮 → 弹/收 popover
    let button: Retained<NSStatusBarButton> = unsafe { msg_send_id![&item, button] };
    unsafe {
        let _: () = msg_send![&button, setTarget: &**delegate];
        let _: () = msg_send![&button, setAction: sel!(togglePopover:)];
    }

    delegate.ivars().status_item.replace(Some(item));
}

/// 按状态把按钮标题设成对应 emoji。Done Notification 期间用 💚 区分(emoji 无法表达深绿)。
pub fn set_light(item: &NSStatusItem, status: AgentStatus, done_notif: bool) {
    let button: Retained<NSStatusBarButton> = unsafe { msg_send_id![item, button] };
    let emoji = if done_notif && status == AgentStatus::Done {
        "💚"
    } else {
        status_emoji(status)
    };
    let title = NSString::from_str(emoji);
    let _: () = unsafe { msg_send![&button, setTitle: &*title] };
}

/// NSTimer scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:
pub fn schedule_tick(delegate: &Retained<AppDelegate>) {
    let interval = agent_light_core::Monitor::poll_interval().as_secs_f64();
    let timer: Retained<NSTimer> = unsafe {
        msg_send_id![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: interval,
            target: &**delegate,
            selector: sel!(tick:),
            userInfo: std::ptr::null_mut::<NSObject>(),
            repeats: Bool::YES,
        ]
    };
    std::mem::forget(timer);
}
