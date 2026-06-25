//! 设置窗口:Wave 2 起步。当前真实控件:「浮窗点击穿透」复选框。
//! 其余(启用 Agent / 轮询间隔 / 主题)为占位,后续补。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::Bool;
use objc2::{class, msg_send, msg_send_id, sel};
use objc2_app_kit::{NSButton, NSTextField, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;

const W: f64 = 380.0;
const H: f64 = 240.0;

pub fn build(delegate: &AppDelegate) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send_id![class!(NSWindow), alloc] };
    // titled(2) | closable(4) = 6
    let window: Retained<NSWindow> = unsafe {
        msg_send_id![
            alloc,
            initWithContentRect: frame,
            styleMask: 6u64,
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let _: () = msg_send![&window, setTitle: &*NSString::from_str("Asig 设置")];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }

    let content: Retained<NSView> = unsafe { msg_send_id![&window, contentView] };

    // 「浮窗点击穿透」复选框(默认勾选 = 穿透)。
    let cb: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        // NSSwitchButton(=3)= macOS 上的复选框外观(无独立 CheckBox 变体)。
        let _: () = msg_send![&cb, setButtonType: 3u64];
        let _: () = msg_send![&cb, setTitle: &*NSString::from_str("浮窗点击穿透")];
        let _: () = msg_send![&cb, setTarget: delegate];
        let _: () = msg_send![&cb, setAction: sel!(toggleClickThrough:)];
        let _: () = msg_send![&cb, setState: 1i64]; // NSControlStateValueOn(初始=穿透)
        let _: () = msg_send![&cb, setFrame: NSRect::new(NSPoint::new(20.0, H - 50.0), NSSize::new(220.0, 24.0))];
        let _: () = msg_send![&content, addSubview: &*cb];
    }

    // 说明文字。
    let label: Retained<NSTextField> = unsafe {
        msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str(
            "勾选=点击穿透(浮窗不挡操作);取消勾选=可用鼠标拖动浮窗位置。\n\n(开发中)\n• 启用 Agent\n• 轮询间隔\n• 颜色主题"
        )]
    };
    unsafe {
        let _: () = msg_send![&label, setFrame: NSRect::new(NSPoint::new(20.0, 20.0), NSSize::new(W - 40.0, H - 80.0))];
        let _: () = msg_send![&content, addSubview: &*label];
    }

    window
}

pub fn show(window: &NSWindow) {
    unsafe {
        let _: () = msg_send![window, center];
        let _: () = msg_send![window, orderFrontRegardless];
    }
}
