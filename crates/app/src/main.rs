//! agent-light macOS 入口。

mod animation;
mod app_delegate;
mod overlay;
mod palette;
mod panel;
mod settings;
mod tray;

use std::cell::RefCell;

use app_delegate::{AppDelegate, AppIvars};
use objc2::rc::Retained;
use objc2::{class, msg_send, msg_send_id};
use objc2_app_kit::NSApplication;

fn main() {
    let app: Retained<NSApplication> =
        unsafe { msg_send_id![class!(NSApplication), sharedApplication] };

    // Phase 2:置顶透明药丸浮窗。
    let (overlay_window, overlay_view) = overlay::build();
    // Phase 2.5:详情 popover + 设置窗口。
    let delegate = AppDelegate::new(AppIvars {
        monitor: agent_light_core::Monitor::default(),
        status_item: RefCell::new(None),
        overlay_window: RefCell::new(Some(overlay_window)),
        overlay_view: RefCell::new(Some(overlay_view)),
        popover: RefCell::new(None),
        settings_window: RefCell::new(None),
        click_through: RefCell::new(true),
        last_sig: RefCell::new(String::new()),
    });
    // popover / 设置窗改为首次点击时懒创建(省常驻内存,压到 <60MB 预算内)。

    tray::build(&delegate);         // 状态栏灯(点击弹 popover)
    tray::schedule_tick(&delegate); // NSTimer 每 2s 轮询内核

    unsafe {
        let _: () = msg_send![&app, setDelegate: &**delegate];
        let _: () = msg_send![&app, run];
    }
}
