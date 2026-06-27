//! 设置窗口:浮窗大小 + 浮窗点击穿透 + 各状态样式(动画/颜色)。
//! 改动经 delegate 的 action 即时存盘并重应用,持久化到
//! ~/Library/Application Support/Asig/settings.json。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{class, msg_send, msg_send_id, sel, DeclaredClass};
use objc2_app_kit::{NSApplication, NSButton, NSPopUpButton, NSSlider, NSTextField, NSView, NSWindow};
use objc2_foundation::{CGFloat, NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{Anim, Color, StyleKey};

use crate::app_delegate::AppDelegate;
use crate::palette::{anim_name, color_name};

const W: CGFloat = 440.0;
const H: CGFloat = 540.0;

pub const ANIM_ORDER: [Anim; 4] = [Anim::Steady, Anim::Pulse, Anim::Blink, Anim::Ripple];
pub const COLOR_ORDER: [Color; 6] =
    [Color::Green, Color::DarkGreen, Color::Yellow, Color::Amber, Color::Red, Color::Purple];

fn state_name(k: StyleKey) -> &'static str {
    match k {
        StyleKey::Done => "Done · 完成",
        StyleKey::Working => "Working · 运行",
        StyleKey::NeedsDeci => "NeedsDeci · 待决策",
        StyleKey::Error => "Error · 报错",
        StyleKey::Offline => "Offline · 离线",
        StyleKey::DoneNotif => "Done-Notif · 完成通知",
    }
}

pub fn build(delegate: &AppDelegate) -> Retained<NSWindow> {
    // 快照当前设置,用作各控件的初始值。
    let (dot, rows): (f64, Vec<(usize, usize)>) = {
        let s = delegate.ivars().settings.borrow();
        let dot = s.dot_size as f64;
        let rows = StyleKey::ALL
            .iter()
            .map(|&k| {
                let style = s.style_for(k); // 含缺省回退
                (
                    ANIM_ORDER.iter().position(|a| *a == style.anim).unwrap_or(0),
                    COLOR_ORDER.iter().position(|c| *c == style.color).unwrap_or(0),
                )
            })
            .collect();
        (dot, rows)
    };
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send_id![class!(NSWindow), alloc] };
    // titled(2) | closable(4) | miniaturizable(1) = 7;可缩放感更自然
    let window: Retained<NSWindow> = unsafe {
        msg_send_id![
            alloc,
            initWithContentRect: frame,
            styleMask: 7u64,
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let _: () = msg_send![&window, setTitle: &*NSString::from_str("Asig 设置")];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }

    let content: Retained<NSView> = unsafe { msg_send_id![&window, contentView] };

    // —— 浮窗大小 ——
    add_label(&content, NSRect::new(NSPoint::new(20.0, 496.0), NSSize::new(160.0, 20.0)), "浮窗大小");
    let alloc: Allocated<NSSlider> = unsafe { msg_send_id![class!(NSSlider), alloc] };
    let slider: Retained<NSSlider> = unsafe {
        msg_send_id![alloc, initWithFrame: NSRect::new(NSPoint::new(20.0, 468.0), NSSize::new(400.0, 22.0))]
    };
    unsafe {
        let _: () = msg_send![&slider, setMinValue: 8.0f64];
        let _: () = msg_send![&slider, setMaxValue: 40.0f64];
        let _: () = msg_send![&slider, setDoubleValue: dot];
        let _: () = msg_send![&slider, setContinuous: Bool::YES];
        let _: () = msg_send![&slider, setTarget: delegate];
        let _: () = msg_send![&slider, setAction: sel!(changeSize:)];
        let _: () = msg_send![&content, addSubview: &*slider];
    }

    // —— 浮窗点击穿透(与 Drop-down「锁定」同步同一开关 toggleClickThrough:) ——
    let cb_click: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    let click_on = *delegate.ivars().click_through.borrow();
    unsafe {
        let _: () = msg_send![&cb_click, setButtonType: 3u64]; // NSSwitchButton 圆角勾选
        let _: () = msg_send![&cb_click, setTitle: &*NSString::from_str("浮窗点击穿透(取消则可用鼠标拖动)")];
        let _: () = msg_send![&cb_click, setState: if click_on { 1i64 } else { 0 }];
        let _: () = msg_send![&cb_click, setTarget: delegate];
        let _: () = msg_send![&cb_click, setAction: sel!(toggleClickThrough:)];
        let _: () = msg_send![&cb_click, setFrame: NSRect::new(NSPoint::new(20.0, 445.0), NSSize::new(400.0, 22.0))];
        let _: () = msg_send![&content, addSubview: &*cb_click];
    }

    // —— 各状态样式 ——
    add_label(&content, NSRect::new(NSPoint::new(20.0, 424.0), NSSize::new(200.0, 20.0)), "各状态样式");
    add_label(&content, NSRect::new(NSPoint::new(150.0, 400.0), NSSize::new(100.0, 16.0)), "动画");
    add_label(&content, NSRect::new(NSPoint::new(290.0, 400.0), NSSize::new(100.0, 16.0)), "颜色");

    let anim_items: Vec<Retained<NSString>> =
        ANIM_ORDER.iter().map(|a| NSString::from_str(anim_name(*a))).collect();
    let color_items: Vec<Retained<NSString>> =
        COLOR_ORDER.iter().map(|c| NSString::from_str(color_name(*c))).collect();

    for (i, (anim_sel, color_sel)) in rows.iter().enumerate() {
        let y = 224.0 + (5 - i) as CGFloat * 30.0; // 6 行自上而下:Done 在顶,Done-Notif 在底
        add_label(
            &content,
            NSRect::new(NSPoint::new(20.0, y + 4.0), NSSize::new(130.0, 20.0)),
            state_name(StyleKey::ALL[i]),
        );
        add_style_popup(
            &content,
            NSRect::new(NSPoint::new(150.0, y), NSSize::new(120.0, 26.0)),
            &anim_items,
            *anim_sel,
            (i as i64) * 2, // tag: field 0 = 动画
            delegate,
        );
        add_style_popup(
            &content,
            NSRect::new(NSPoint::new(290.0, y), NSSize::new(120.0, 26.0)),
            &color_items,
            *color_sel,
            (i as i64) * 2 + 1, // tag: field 1 = 颜色
            delegate,
        );
    }

    window
}

/// 一个只读文本标签。
fn add_label(content: &Retained<NSView>, frame: NSRect, text: &str) {
    let label: Retained<NSTextField> =
        unsafe { msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str(text)] };
    unsafe {
        let _: () = msg_send![&label, setFrame: frame];
        let _: () = msg_send![&**content, addSubview: &*label];
    }
}

/// 一个状态下拉(动画或颜色),action 统一 changeStyle:,tag 编码 (state, field)。
fn add_style_popup(
    content: &Retained<NSView>,
    frame: NSRect,
    items: &[Retained<NSString>],
    selected: usize,
    tag: i64,
    delegate: &AppDelegate,
) {
    let alloc: Allocated<NSPopUpButton> = unsafe { msg_send_id![class!(NSPopUpButton), alloc] };
    let pop: Retained<NSPopUpButton> =
        unsafe { msg_send_id![alloc, initWithFrame: frame, pullsDown: Bool::NO] };
    for it in items {
        unsafe {
            let _: () = msg_send![&pop, addItemWithTitle: &**it];
        }
    }
    unsafe {
        let _: () = msg_send![&pop, selectItemAtIndex: selected as i64];
        let _: () = msg_send![&pop, setTag: tag];
        let _: () = msg_send![&pop, setTarget: delegate];
        let _: () = msg_send![&pop, setAction: sel!(changeStyle:)];
        let _: () = msg_send![&**content, addSubview: &*pop];
    }
}

pub fn show(window: &NSWindow) {
    unsafe {
        let app: Retained<NSApplication> = msg_send_id![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&app, activateIgnoringOtherApps: Bool::YES];
        let _: () = msg_send![window, center];
        let _: () = msg_send![window, makeKeyAndOrderFront: std::ptr::null_mut::<NSObject>()];
    }
}
