//! 详情 popover:点状态栏 Asig 弹出。含会话列表 + 设置/退出按钮(Stats 风格)。

use agent_light_core::Snapshot;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{class, msg_send, msg_send_id, sel};
use objc2_app_kit::{NSButton, NSColor, NSFont, NSScreen, NSTextField, NSView, NSWindow};
use objc2_foundation::{CGFloat, NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;
use crate::palette::status_emoji;

const W: CGFloat = 280.0;
const H: CGFloat = 220.0;

pub struct Popover {
    pub window: Retained<NSWindow>,
    label: Retained<NSTextField>,
}

pub fn build(delegate: &AppDelegate) -> Popover {
    // 在 build 时就把窗口定位到菜单栏下方(此时窗口尚未被 KVO 子类化,
    // 可安全用 initWithContentRect 传 NSRect;避免后续对 NSKVONotifying_NSWindow
    // 发结构体参数消息被 objc2 拒绝)。
    let (x, y) = top_right(W, H);
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send_id![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send_id![
            alloc,
            initWithContentRect: frame,
            styleMask: 0u64, // borderless
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let bg: Retained<NSColor> = msg_send_id![class!(NSColor), windowBackgroundColor];
        let _: () = msg_send![&window, setOpaque: Bool::NO];
        let _: () = msg_send![&window, setBackgroundColor: &*bg];
        let _: () = msg_send![&window, setHasShadow: Bool::YES];
        let _: () = msg_send![&window, setLevel: 3i64]; // floating
        let _: () = msg_send![&window, setHidesOnDeactivate: Bool::YES];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }

    let content: Retained<NSView> = unsafe { msg_send_id![&window, contentView] };

    // 标题
    let title: Retained<NSTextField> = unsafe {
        msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str("Asig")]
    };
    unsafe {
        let _: () = msg_send![&title, setFrame: NSRect::new(NSPoint::new(16.0, H - 34.0), NSSize::new(W - 32.0, 22.0))];
        let font: Retained<NSFont> = msg_send_id![class!(NSFont), boldSystemFontOfSize: 14.0];
        let _: () = msg_send![&title, setFont: &*font];
        let _: () = msg_send![&content, addSubview: &*title];
    }

    // 会话列表(label)
    let label: Retained<NSTextField> = unsafe {
        msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str("(无会话)")]
    };
    unsafe {
        let _: () = msg_send![&label, setFrame: NSRect::new(NSPoint::new(16.0, 60.0), NSSize::new(W - 32.0, H - 100.0))];
        let font: Retained<NSFont> = msg_send_id![class!(NSFont), systemFontOfSize: 12.0];
        let _: () = msg_send![&label, setFont: &*font];
        let _: () = msg_send![&content, addSubview: &*label];
    }

    // 设置按钮
    let btn_settings: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn_settings, setFrame: NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(96.0, 30.0))];
        let _: () = msg_send![&btn_settings, setTitle: &*NSString::from_str("设置…")];
        let _: () = msg_send![&btn_settings, setTarget: delegate];
        let _: () = msg_send![&btn_settings, setAction: sel!(openSettings:)];
        let _: () = msg_send![&content, addSubview: &*btn_settings];
    }

    // 退出按钮
    let btn_quit: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn_quit, setFrame: NSRect::new(NSPoint::new(W - 16.0 - 96.0, 16.0), NSSize::new(96.0, 30.0))];
        let _: () = msg_send![&btn_quit, setTitle: &*NSString::from_str("退出")];
        let _: () = msg_send![&btn_quit, setTarget: delegate];
        let _: () = msg_send![&btn_quit, setAction: sel!(quit:)];
        let _: () = msg_send![&content, addSubview: &*btn_quit];
    }

    Popover { window, label }
}

/// 显示/隐藏(窗口已在 build 时定位好,这里只做 order)。
pub fn toggle(p: &Popover) {
    let visible: bool = unsafe { msg_send![&p.window, isVisible] };
    if visible {
        let _: () = unsafe { msg_send![&p.window, orderOut: std::ptr::null_mut::<NSObject>()] };
    } else {
        let _: () = unsafe { msg_send![&p.window, orderFrontRegardless] };
    }
}

fn top_right(w: CGFloat, h: CGFloat) -> (CGFloat, CGFloat) {
    let screen: Retained<NSScreen> = unsafe { msg_send_id![class!(NSScreen), mainScreen] };
    let f: NSRect = unsafe { msg_send![&screen, visibleFrame] };
    (f.origin.x + f.size.width - w - 12.0, f.origin.y + f.size.height - h - 8.0)
}

/// 用最新快照刷新会话列表。
pub fn update_label(p: &Popover, snap: &Snapshot) {
    let text = if snap.sessions.is_empty() {
        "(无活跃会话)".to_string()
    } else {
        snap.sessions
            .iter()
            .map(|s| {
                format!(
                    "{} {:?} · {}",
                    status_emoji(s.status),
                    s.kind,
                    s.project.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    unsafe {
        let _: () = msg_send![&p.label, setStringValue: &*NSString::from_str(&text)];
    }
}
