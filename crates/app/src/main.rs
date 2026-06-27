//! agent-light macOS 入口。

mod app_delegate;
mod overlay;
mod palette;
mod panel;
mod settings;
mod tray;

use std::cell::RefCell;

use app_delegate::{AppDelegate, AppIvars};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, Sel};
use objc2::{class, msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::NSTimer;

fn main() {
    let app: Retained<NSApplication> =
        unsafe { msg_send![class!(NSApplication), sharedApplication] };

    // 启动即加载用户设置(灯大小 + 各状态样式)。
    let settings = agent_light_core::Settings::load();

    // Phase 2:置顶透明药丸浮窗(按设置里的圆点大小 + 上次记忆的位置初始化)。
    let (overlay_window, overlay_view) = overlay::build(settings.dot_size, settings.light_pos);
    // Phase 2.5:详情 popover + 设置窗口。
    let delegate = AppDelegate::new(AppIvars {
        monitor: agent_light_core::Monitor::default(),
        status_item: RefCell::new(None),
        overlay_window: RefCell::new(Some(overlay_window)),
        overlay_view: RefCell::new(Some(overlay_view)),
        popover: RefCell::new(None),
        settings_window: RefCell::new(None),
        click_through: RefCell::new(true),
        settings: RefCell::new(settings),
        last_sig: RefCell::new(String::new()),
    });
    // popover / 设置窗改为首次点击时懒创建(省常驻内存,压到 <60MB 预算内)。

    tray::build(&delegate); // 状态栏 Signal Icon(点击弹 Drop-down)
    tray::schedule_tick(&delegate); // NSTimer 每 3s 轮询内核

    // 开发/测试钩子:绕过「合成点击无法触发菜单栏 NSStatusItem」的 macOS 限制——
    // 延迟 0.5s(run loop 起来、图标布局好之后)直接打开面板,便于自动截图核对。
    // 生产运行不设这两个环境变量。
    unsafe {
        let open_after = |sel_name: Sel| {
            let timer: Retained<NSTimer> = msg_send![
                class!(NSTimer),
                scheduledTimerWithTimeInterval: 0.5f64,
                target: &**delegate,
                selector: sel_name,
                userInfo: std::ptr::null_mut::<NSObject>(),
                repeats: Bool::NO,
            ];
            std::mem::forget(timer);
        };
        if std::env::var("ASIG_PANEL").is_ok() {
            open_after(sel!(togglePopover:));
        }
        if std::env::var("ASIG_SETTINGS").is_ok() {
            open_after(sel!(openSettings:));
        }
    }

    unsafe {
        let _: () = msg_send![&app, setDelegate: &**delegate];
        let _: () = msg_send![&app, run];
    }
}
