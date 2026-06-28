//! Drop-down Panel:单击菜单栏 Signal Icon 后弹出的原生 NSPopover。内容 = 标题 + 三按钮
//! (设置 / 锁定 / 退出)+ 会话列表。NSPopover 自带圆角 + 箭头 + vibrancy 材质 + 失焦自动关
//! (behavior=.transient),故不再自绘 borderless 窗 / CardView / 手算定位。

use agent_light_core::Snapshot;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{DefinedClass, MainThreadMarker, class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSFont, NSPopover, NSStatusBarButton, NSTextField, NSView,
    NSViewController,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;
use crate::palette::status_emoji;

pub const PANEL_W: f64 = 280.0;
pub const PANEL_H: f64 = 220.0;

pub struct Popover {
    popover: Retained<NSPopover>,
    label: Retained<NSTextField>,
}

/// 构建 popover(不显示):内容视图(标题 + 三按钮 + 会话列表)→ VC → NSPopover(transient)。
pub fn build(delegate: &AppDelegate) -> Popover {
    let mtm = MainThreadMarker::new().expect("panel build 须在主线程");
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_W, PANEL_H));
    let content: Retained<NSView> = unsafe { msg_send![class!(NSView), new] };
    unsafe {
        let _: () = msg_send![&content, setFrame: frame];
    }

    // 标题
    add_label(
        &content,
        NSRect::new(
            NSPoint::new(16.0, PANEL_H - 28.0),
            NSSize::new(PANEL_W - 32.0, 18.0),
        ),
        "Asig",
        true,
    );

    // —— 顶部三按钮(左→右):设置 / 锁定 / 退出。三按钮均 76pt 宽、间距 10pt(248pt 可用)——
    let _ = add_button(
        &content,
        NSRect::new(NSPoint::new(16.0, PANEL_H - 64.0), NSSize::new(76.0, 30.0)),
        "设置",
        delegate,
        sel!(openSettings:),
    );
    let locked = *delegate.ivars().click_through.borrow(); // 锁定 = 不可拖动 = click_through
    let btn_lock = add_button(
        &content,
        NSRect::new(NSPoint::new(102.0, PANEL_H - 64.0), NSSize::new(76.0, 30.0)),
        "锁定",
        delegate,
        sel!(toggleClickThrough:),
    );
    unsafe {
        let _: () = msg_send![&btn_lock, setButtonType: 3u64]; // NSSwitchButton 圆角勾选
        let _: () = msg_send![&btn_lock, setState: if locked { 1i64 } else { 0 }];
    }
    let _ = add_button(
        &content,
        NSRect::new(
            NSPoint::new(PANEL_W - 16.0 - 76.0, PANEL_H - 64.0),
            NSSize::new(76.0, 30.0),
        ),
        "退出",
        delegate,
        sel!(quit:),
    );

    // 会话列表
    let label: Retained<NSTextField> = unsafe {
        msg_send![class!(NSTextField), labelWithString: &*NSString::from_str("(无会话)")]
    };
    unsafe {
        let font: Retained<NSFont> = msg_send![class!(NSFont), systemFontOfSize: 12.0f64];
        let _: () = msg_send![&label, setFont: &*font];
        let _: () = msg_send![
            &label,
            setFrame: NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(PANEL_W - 32.0, PANEL_H - 96.0))
        ];
        let _: () = msg_send![&content, addSubview: &*label];
    }

    // 包 VC → NSPopover(transient:失焦自动关)
    let vc = NSViewController::new(mtm);
    unsafe {
        let _: () = msg_send![&vc, setView: &*content];
    }
    let popover = NSPopover::new(mtm);
    // ASIG_NO_HIDE(dev):behavior=0(ApplicationDefined)不随失焦关,便于截图;默认 1(Transient)。
    let behavior: i64 = if std::env::var("ASIG_NO_HIDE").is_ok() {
        0
    } else {
        1
    };
    unsafe {
        let _: () = msg_send![&popover, setBehavior: behavior]; // 0=ApplicationDefined / 1=Transient
        let _: () = msg_send![&popover, setContentSize: NSSize::new(PANEL_W, PANEL_H)];
        let _: () = msg_send![&popover, setContentViewController: Some(&*vc)];
    }

    Popover { popover, label }
}

/// 锚在状态栏按钮下方弹出 popover。
pub fn show(p: &Popover, button: &NSStatusBarButton) {
    let rect: NSRect = unsafe { msg_send![button, bounds] };
    unsafe {
        // NSApplication 激活(transient popover 否则可能不显示)
        let app: Retained<NSApplication> = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&app, activateIgnoringOtherApps: Bool::YES];
        let _: () = msg_send![
            &p.popover,
            showRelativeToRect: rect,
            ofView: button,
            preferredEdge: 1i64 // NSMinYEdge(下方)
        ];
    }
}

pub fn is_visible(p: &Popover) -> bool {
    p.popover.isShown()
}

pub fn hide(p: &Popover) {
    let _: () = unsafe {
        msg_send![&p.popover, performClose: std::ptr::null_mut::<objc2::runtime::NSObject>()]
    };
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

fn add_label(content: &Retained<NSView>, frame: NSRect, text: &str, bold: bool) {
    let label: Retained<NSTextField> =
        unsafe { msg_send![class!(NSTextField), labelWithString: &*NSString::from_str(text)] };
    unsafe {
        if bold {
            let font: Retained<NSFont> = msg_send![class!(NSFont), boldSystemFontOfSize: 14.0f64];
            let _: () = msg_send![&label, setFont: &*font];
        }
        let _: () = msg_send![&label, setFrame: frame];
        let _: () = msg_send![content, addSubview: &*label];
    }
}

/// 建一个普通按钮:frame / title / target / action 一次配齐并加到 content;返回它供进一步定制。
fn add_button(
    content: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    delegate: &AppDelegate,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str(title)];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: action];
        let _: () = msg_send![content, addSubview: &*btn];
    }
    btn
}
