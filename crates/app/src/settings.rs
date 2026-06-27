//! 设置窗口(左侧栏导航)。
//! 左栏:常规 + 6 个状态 tab(按 DEV.md「Left Side Tabs」顺序)+ 底部图标行
//! (关于 functional;访问官网/调试/捐赠/退出 占位禁用)。右区:8 个 pane 切换。
//! 改动经 delegate 的 action 即时存盘并重应用,持久化到
//! ~/Library/Application Support/Asig/settings.json。
//!
//! 控件 tag 协议:
//! - 侧栏项 id:0=常规, 1..6=状态(见 STATE_TABS), 7=关于, 8..11=占位图标。
//! - 状态控件 tag = tab_id*100 + field(field:1=颜色 2=动画 3=速度滑块 4=速度标签)。
//!   由 `parse_control_tag` 解码回 (StyleKey, field)。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, Sel};
use objc2::{DefinedClass, class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSFont, NSPopUpButton, NSSlider, NSTextField, NSView, NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{Anim, Color, StyleKey};

use crate::app_delegate::AppDelegate;
use crate::palette::{anim_name, color_name};

const W: CGFloat = 680.0;
const H: CGFloat = 460.0;
const SIDEBAR_W: CGFloat = 170.0;
const CONTENT_X: CGFloat = SIDEBAR_W;
const CONTENT_W: CGFloat = W - SIDEBAR_W;

/// 关于页显示的仓库链接(占位,改成真实仓库)。
const GITHUB_URL: &str = "https://github.com/koki/Asig";

pub const ANIM_ORDER: [Anim; 3] = [Anim::Steady, Anim::Pulse, Anim::Ripple];
pub const COLOR_ORDER: [Color; 6] = [
    Color::Green,
    Color::DarkGreen,
    Color::Yellow,
    Color::Amber,
    Color::Red,
    Color::Purple,
];

/// 轮询间隔下拉的可选项(ms)。index ↔ 选中项。
pub const POLL_PRESETS_MS: [u32; 5] = [1000, 2000, 3000, 5000, 10000];

// ---- 侧栏 tab id ----
pub const TAB_GENERAL: i64 = 0;
pub const TAB_ABOUT: i64 = 7;

/// 状态 tab(DEV.md「Left Side Tabs」顺序):1=DoneNotif … 6=Offline。
const STATE_TABS: [(i64, StyleKey, &str); 6] = [
    (1, StyleKey::DoneNotif, "Done-Notif · 完成通知"),
    (2, StyleKey::Done, "Done · 完成"),
    (3, StyleKey::Working, "Working · 运行"),
    (4, StyleKey::NeedsDeci, "NeedsDeci · 待决策"),
    (5, StyleKey::Error, "Error · 报错"),
    (6, StyleKey::Offline, "Offline · 离线"),
];

// 状态控件 field 编码(tag = tab_id*100 + field)。
pub const F_COLOR: i64 = 1;
pub const F_ANIM: i64 = 2;
pub const F_SPEED: i64 = 3;
pub const F_SPEED_LABEL: i64 = 4;

/// 侧栏 tab id → StyleKey(仅状态 tab)。
pub fn stylekey_of_tab(tab: i64) -> Option<StyleKey> {
    STATE_TABS
        .iter()
        .find(|(t, _, _)| *t == tab)
        .map(|(_, k, _)| *k)
}

/// StyleKey → 对应的侧栏 tab id。
fn tab_of_key(key: StyleKey) -> i64 {
    STATE_TABS
        .iter()
        .find(|(_, k, _)| *k == key)
        .map(|(t, _, _)| *t)
        .unwrap_or(TAB_GENERAL)
}

/// 状态控件 tag → (StyleKey, field)。
pub fn parse_control_tag(tag: i64) -> Option<(StyleKey, i64)> {
    stylekey_of_tab(tag / 100).map(|k| (k, tag % 100))
}

/// content view 里按 tag 找子视图(pane / 控件)。找不到 → None。
pub fn view_with_tag(view: &Retained<NSView>, tag: i64) -> Option<Retained<NSView>> {
    unsafe { msg_send![view, viewWithTag: tag] }
}

/// period_ms → Hz(Steady 的 0 返回 0.0)。
fn hz_of(period_ms: u32) -> f64 {
    if period_ms == 0 {
        0.0
    } else {
        1000.0 / period_ms as f64
    }
}

fn poll_preset_index(ms: u32) -> usize {
    POLL_PRESETS_MS.iter().position(|&p| p == ms).unwrap_or(2) // 缺省 3s
}

pub fn build(delegate: &AppDelegate) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: 7u64, // titled | closable | miniaturizable
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let _: () = msg_send![&window, setTitle: &*NSString::from_str("Asig 设置")];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }
    let content: Retained<NSView> = unsafe { msg_send![&window, contentView] };

    // —— 左侧栏 ——
    let sidebar = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(SIDEBAR_W, H),
    ));
    build_sidebar(&sidebar, delegate);
    unsafe {
        let _: () = msg_send![&content, addSubview: &*sidebar];
    }

    // —— 右侧内容区:8 个 pane,默认显示「常规」——
    let content_area = new_view(NSRect::new(
        NSPoint::new(CONTENT_X, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let panes: Vec<Retained<NSView>> = vec![
        build_general_pane(delegate),
        build_state_pane(delegate, StyleKey::DoneNotif),
        build_state_pane(delegate, StyleKey::Done),
        build_state_pane(delegate, StyleKey::Working),
        build_state_pane(delegate, StyleKey::NeedsDeci),
        build_state_pane(delegate, StyleKey::Error),
        build_state_pane(delegate, StyleKey::Offline),
        build_about_pane(),
    ];
    // NSView 无 setTag(只有 viewWithTag/tag),故 pane 按索引切换、不打 tag。
    // 默认显示索引 0(常规),其余隐藏。
    for (i, pane) in panes.iter().enumerate() {
        unsafe {
            let _: () = msg_send![pane, setHidden: Bool::new(i != 0)];
            let _: () = msg_send![&content_area, addSubview: &**pane];
        }
    }
    unsafe {
        let _: () = msg_send![&content, addSubview: &*content_area];
    }

    *delegate.ivars().settings_sidebar.borrow_mut() = Some(sidebar);
    *delegate.ivars().settings_content.borrow_mut() = Some(content_area);
    *delegate.ivars().settings_panes.borrow_mut() = Some(panes);
    *delegate.ivars().settings_selected.borrow_mut() = TAB_GENERAL;
    update_tab_prefixes(delegate, TAB_GENERAL);

    window
}

/// 侧栏:顶部 tab 列表(常规 + 6 状态)+ 底部图标行(关于 functional,其余占位禁用)。
fn build_sidebar(sidebar: &Retained<NSView>, delegate: &AppDelegate) {
    // 顶部 tab(自上而下)
    let tabs: [(i64, &str); 7] = [
        (TAB_GENERAL, "常规"),
        (1, "Done-Notif · 完成通知"),
        (2, "Done · 完成"),
        (3, "Working · 运行"),
        (4, "NeedsDeci · 待决策"),
        (5, "Error · 报错"),
        (6, "Offline · 离线"),
    ];
    for (i, (tag, label)) in tabs.iter().enumerate() {
        let y = H - 16.0 - (i as CGFloat + 1.0) * 32.0;
        add_plain_button(
            sidebar,
            NSRect::new(NSPoint::new(8.0, y), NSSize::new(SIDEBAR_W - 16.0, 28.0)),
            label,
            *tag,
            sel!(switchSettingsTab:),
            delegate,
        );
    }

    // 底部图标行(L→R:关于 functional / 其余占位禁用)
    let icons: [(&str, i64, bool); 5] = [
        ("ℹ︎", TAB_ABOUT, true),
        ("🌐", 8, false),
        ("🐛", 9, false),
        ("💝", 10, false),
        ("⏻", 11, false),
    ];
    let icon_w = (SIDEBAR_W - 16.0) / icons.len() as CGFloat;
    for (i, (sym, tag, enabled)) in icons.iter().enumerate() {
        let x = 8.0 + i as CGFloat * icon_w;
        let btn = add_plain_button(
            sidebar,
            NSRect::new(NSPoint::new(x, 8.0), NSSize::new(icon_w, 28.0)),
            sym,
            *tag,
            sel!(switchSettingsTab:),
            delegate,
        );
        if !*enabled {
            unsafe {
                let _: () = msg_send![&btn, setEnabled: Bool::NO];
            }
        }
    }
}

/// 切换选中 tab:重设顶部文本 tab 的「▸」前缀(选中项加、其余去)。
pub fn update_tab_prefixes(delegate: &AppDelegate, selected: i64) {
    let Some(sidebar) = delegate.ivars().settings_sidebar.borrow().as_ref().cloned() else {
        return;
    };
    let tabs: [(i64, &str); 7] = [
        (TAB_GENERAL, "常规"),
        (1, "Done-Notif · 完成通知"),
        (2, "Done · 完成"),
        (3, "Working · 运行"),
        (4, "NeedsDeci · 待决策"),
        (5, "Error · 报错"),
        (6, "Offline · 离线"),
    ];
    for (tag, label) in tabs {
        let Some(b) = view_with_tag(&sidebar, tag) else {
            continue;
        };
        let title = if tag == selected {
            format!("▸ {label}")
        } else {
            label.to_string()
        };
        unsafe {
            let _: () = msg_send![&b, setTitle: &*NSString::from_str(&title)];
        }
    }
}

// ---- 各 pane ----

fn build_general_pane(delegate: &AppDelegate) -> Retained<NSView> {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let pad = 24.0;
    let mut y = H - 44.0;

    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(160.0, 20.0)),
        "常规",
        false,
        true,
    );
    y -= 38.0;

    // 浮窗大小
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(160.0, 20.0)),
        "浮窗大小",
        false,
        false,
    );
    y -= 30.0;
    let dot = delegate.ivars().settings.borrow().dot_size as f64;
    add_slider(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        8.0,
        40.0,
        dot,
        sel!(changeSize:),
        delegate,
    );
    y -= 44.0;

    // 浮窗点击穿透
    let click_on = *delegate.ivars().click_through.borrow();
    add_checkbox(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        "浮窗点击穿透(取消则可用鼠标拖动)",
        click_on,
        sel!(toggleClickThrough:),
        delegate,
    );
    y -= 36.0;

    // 轮询间隔
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(120.0, 20.0)),
        "轮询间隔",
        false,
        false,
    );
    let poll_ms = delegate.ivars().settings.borrow().poll_interval_ms;
    add_popup(
        &pane,
        NSRect::new(NSPoint::new(pad + 130.0, y - 2.0), NSSize::new(120.0, 26.0)),
        &["1 秒", "2 秒", "3 秒", "5 秒", "10 秒"],
        poll_preset_index(poll_ms),
        sel!(changePollInterval:),
        delegate,
        0,
    );
    y -= 40.0;

    // 开机启动(占位,暂未实现)
    let launch = add_checkbox(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        "开机启动(待实现)",
        false,
        sel!(noop:),
        delegate,
    );
    unsafe {
        let _: () = msg_send![&launch, setEnabled: Bool::NO];
    }

    pane
}

fn build_state_pane(delegate: &AppDelegate, key: StyleKey) -> Retained<NSView> {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let pad = 24.0;
    let tab = tab_of_key(key);
    let base = tab * 100;
    let mut y = H - 48.0;

    let name = STATE_TABS
        .iter()
        .find(|(_, k, _)| *k == key)
        .map(|(_, _, n)| *n)
        .unwrap_or("?");
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        name,
        false,
        true,
    );
    y -= 44.0;

    let style = delegate.ivars().settings.borrow().style_for(key);

    // 颜色
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(80.0, 20.0)),
        "颜色",
        false,
        false,
    );
    let color_sel = COLOR_ORDER
        .iter()
        .position(|c| *c == style.color)
        .unwrap_or(0);
    let color_items: Vec<&str> = COLOR_ORDER.iter().map(|c| color_name(*c)).collect();
    add_popup(
        &pane,
        NSRect::new(NSPoint::new(pad + 90.0, y - 2.0), NSSize::new(150.0, 26.0)),
        &color_items,
        color_sel,
        sel!(changeStyle:),
        delegate,
        base + F_COLOR,
    );
    y -= 40.0;

    // 动画
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(80.0, 20.0)),
        "动画",
        false,
        false,
    );
    let anim_sel = ANIM_ORDER
        .iter()
        .position(|a| *a == style.anim)
        .unwrap_or(0);
    let anim_items: Vec<&str> = ANIM_ORDER.iter().map(|a| anim_name(*a)).collect();
    add_popup(
        &pane,
        NSRect::new(NSPoint::new(pad + 90.0, y - 2.0), NSSize::new(150.0, 26.0)),
        &anim_items,
        anim_sel,
        sel!(changeStyle:),
        delegate,
        base + F_ANIM,
    );
    y -= 44.0;

    // 速度(Hz)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(80.0, 20.0)),
        "速度",
        false,
        false,
    );
    let is_steady = style.anim == Anim::Steady;
    let hz = if is_steady {
        1.0
    } else {
        hz_of(style.period_ms).clamp(0.3, 5.0)
    };
    let slider = add_slider(
        &pane,
        NSRect::new(NSPoint::new(pad + 90.0, y), NSSize::new(220.0, 22.0)),
        0.3,
        5.0,
        hz,
        sel!(changeSpeed:),
        delegate,
    );
    set_tag(&slider, base + F_SPEED);
    let speed_text = if is_steady {
        "—".to_string()
    } else {
        format!("{:.1} Hz", hz)
    };
    let speed_lbl = add_text(
        &pane,
        NSRect::new(NSPoint::new(pad + 90.0 + 230.0, y), NSSize::new(70.0, 20.0)),
        &speed_text,
        false,
        false,
    );
    set_tag(&speed_lbl, base + F_SPEED_LABEL);
    if is_steady {
        unsafe {
            let _: () = msg_send![&slider, setEnabled: Bool::NO];
        }
    }

    pane
}

fn build_about_pane() -> Retained<NSView> {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, H - 150.0), NSSize::new(CONTENT_W, 30.0)),
        "Asig",
        true,
        true,
    );
    add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, H - 190.0), NSSize::new(CONTENT_W, 20.0)),
        &format!("版本 {}", env!("CARGO_PKG_VERSION")),
        true,
        false,
    );
    add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, H - 220.0), NSSize::new(CONTENT_W, 20.0)),
        GITHUB_URL,
        true,
        false,
    );
    pane
}

// ---- 控件工厂 ----

fn new_view(frame: NSRect) -> Retained<NSView> {
    let v: Retained<NSView> = unsafe { msg_send![class!(NSView), new] };
    unsafe {
        let _: () = msg_send![&v, setFrame: frame];
    }
    v
}

fn set_tag<T: objc2::Message>(view: &Retained<T>, tag: i64) {
    unsafe {
        let _: () = msg_send![view, setTag: tag];
    }
}

/// 无边框按钮(侧栏 tab / 图标行通用)。
fn add_plain_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    tag: i64,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setBordered: Bool::NO];
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str(title)];
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: action];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

fn add_text(
    pane: &Retained<NSView>,
    frame: NSRect,
    text: &str,
    center: bool,
    bold: bool,
) -> Retained<NSTextField> {
    let label: Retained<NSTextField> =
        unsafe { msg_send![class!(NSTextField), labelWithString: &*NSString::from_str(text)] };
    unsafe {
        if bold {
            let font: Retained<NSFont> = msg_send![class!(NSFont), boldSystemFontOfSize: 14.0f64];
            let _: () = msg_send![&label, setFont: &*font];
        }
        if center {
            let _: () = msg_send![&label, setAlignment: 2i64]; // NSTextAlignmentCenter
        }
        let _: () = msg_send![&label, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*label];
    }
    label
}

fn add_slider(
    pane: &Retained<NSView>,
    frame: NSRect,
    min: f64,
    max: f64,
    val: f64,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSSlider> {
    let alloc: Allocated<NSSlider> = unsafe { msg_send![class!(NSSlider), alloc] };
    let slider: Retained<NSSlider> = unsafe { msg_send![alloc, initWithFrame: frame] };
    unsafe {
        let _: () = msg_send![&slider, setMinValue: min];
        let _: () = msg_send![&slider, setMaxValue: max];
        let _: () = msg_send![&slider, setDoubleValue: val];
        let _: () = msg_send![&slider, setContinuous: Bool::YES];
        let _: () = msg_send![&slider, setTarget: delegate];
        let _: () = msg_send![&slider, setAction: action];
        let _: () = msg_send![&**pane, addSubview: &*slider];
    }
    slider
}

fn add_checkbox(
    pane: &Retained<NSView>,
    frame: NSRect,
    text: &str,
    on: bool,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setButtonType: 3u64]; // NSSwitchButton
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str(text)];
        let _: () = msg_send![&btn, setState: if on { 1i64 } else { 0 }];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: action];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

fn add_popup(
    pane: &Retained<NSView>,
    frame: NSRect,
    items: &[&str],
    selected: usize,
    action: Sel,
    delegate: &AppDelegate,
    tag: i64,
) -> Retained<NSPopUpButton> {
    let alloc: Allocated<NSPopUpButton> = unsafe { msg_send![class!(NSPopUpButton), alloc] };
    let pop: Retained<NSPopUpButton> =
        unsafe { msg_send![alloc, initWithFrame: frame, pullsDown: Bool::NO] };
    for it in items {
        unsafe {
            let _: () = msg_send![&pop, addItemWithTitle: &*NSString::from_str(it)];
        }
    }
    unsafe {
        let _: () = msg_send![&pop, selectItemAtIndex: selected as i64];
        let _: () = msg_send![&pop, setTag: tag];
        let _: () = msg_send![&pop, setTarget: delegate];
        let _: () = msg_send![&pop, setAction: action];
        let _: () = msg_send![&**pane, addSubview: &*pop];
    }
    pop
}

pub fn show(window: &NSWindow) {
    unsafe {
        let app: Retained<NSApplication> = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&app, activateIgnoringOtherApps: Bool::YES];
        let _: () = msg_send![window, center];
        let _: () = msg_send![window, makeKeyAndOrderFront: std::ptr::null_mut::<objc2::runtime::NSObject>()];
    }
}
