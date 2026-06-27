//! 设置窗口(左侧栏导航)。
//! 左栏:General + 6 状态 tab(按 DEV.md「Left Side Tabs」顺序,左对齐;状态 tab = 当前色
//! 圆点 + 英文简称)+ 底部图标行(关于 functional;官网/调试/捐赠/退出 占位禁用,单色 SF Symbol)。
//! 右区:8 个 pane 切换。状态 pane = State Settings Card(Reset + Color 色块单选 + Animation
//! 单选 + Speed Hz)。
//!
//! 控件 tag 协议(仅控件,pane 按 Vec 索引切):
//! - base = tab_id * 1000;sub: COLOR_OFF+i(颜色)、ANIM_OFF+i(动画)、SPEED_OFF、SPEED_LABEL_OFF、RESET_OFF。

use std::collections::HashMap;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, Sel};
use objc2::{DefinedClass, class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSFont, NSImage, NSPopUpButton, NSSlider, NSTextField, NSView,
    NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{Anim, Color, StateStyle, StyleKey};

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

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

pub const TAB_GENERAL: i64 = 0;
pub const TAB_ABOUT: i64 = 7;

/// 状态 tab(DEV.md「Left Side Tabs」顺序);label 为英文简称。
const STATE_TABS: [(i64, StyleKey, &str); 6] = [
    (1, StyleKey::DoneNotif, "DoneNotif"),
    (2, StyleKey::Done, "Done"),
    (3, StyleKey::Working, "Working"),
    (4, StyleKey::NeedsDeci, "NeedsDeci"),
    (5, StyleKey::Error, "Error"),
    (6, StyleKey::Offline, "Offline"),
];

// tag sub 偏移(base = tab_id*1000)。
pub const COLOR_OFF: i64 = 10; // +i(0..6)
pub const ANIM_OFF: i64 = 20; // +i(0..3)
pub const SPEED_OFF: i64 = 30;
pub const SPEED_LABEL_OFF: i64 = 31;
pub const RESET_OFF: i64 = 40;

pub const SPEED_MIN: f64 = 0.2;
pub const SPEED_MAX: f64 = 5.0;
const SWATCH_D: CGFloat = 26.0;

/// 一个状态 pane 的全部控件(类型化引用,便于 reset / 选择变更时批量刷新)。
pub struct StateControls {
    pub color: Vec<Retained<NSButton>>,
    pub anim: Vec<Retained<NSButton>>,
    pub speed: Retained<NSSlider>,
    pub speed_label: Retained<NSTextField>,
}

pub fn stylekey_of_tab(tab: i64) -> Option<StyleKey> {
    STATE_TABS
        .iter()
        .find(|(t, _, _)| *t == tab)
        .map(|(_, k, _)| *k)
}

fn tab_of_key(key: StyleKey) -> i64 {
    STATE_TABS
        .iter()
        .find(|(_, k, _)| *k == key)
        .map(|(t, _, _)| *t)
        .unwrap_or(TAB_GENERAL)
}

/// 控件 tag → (StyleKey, sub)。
pub fn parse_control_tag(tag: i64) -> Option<(StyleKey, i64)> {
    stylekey_of_tab(tag / 1000).map(|k| (k, tag % 1000))
}

fn hz_of(period_ms: u32) -> f64 {
    if period_ms == 0 {
        0.0
    } else {
        1000.0 / period_ms as f64
    }
}

fn poll_preset_index(ms: u32) -> usize {
    POLL_PRESETS_MS.iter().position(|&p| p == ms).unwrap_or(2)
}

/// 单色 SF Symbol 图标(底栏用,template 渲染跟随明暗)。
fn sf_symbol(name: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
        .expect("SF Symbol not found")
}

pub fn build(delegate: &AppDelegate) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: 7u64,
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let _: () = msg_send![&window, setTitle: &*NSString::from_str("Asig")];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }
    let content: Retained<NSView> = unsafe { msg_send![&window, contentView] };

    let sidebar = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(SIDEBAR_W, H),
    ));
    build_sidebar(&sidebar, delegate);
    unsafe {
        let _: () = msg_send![&content, addSubview: &*sidebar];
    }

    let content_area = new_view(NSRect::new(
        NSPoint::new(CONTENT_X, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    // 8 pane:General + 6 状态(各带 StateControls)+ About。按 pane id(=索引)排。
    let mut panes: Vec<Retained<NSView>> = Vec::with_capacity(8);
    let mut controls_map: HashMap<StyleKey, StateControls> = HashMap::new();
    panes.push(build_general_pane(delegate));
    for (_, key, _) in STATE_TABS {
        let (pane, c) = build_state_pane(delegate, key);
        controls_map.insert(key, c);
        panes.push(pane);
    }
    panes.push(build_about_pane());
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
    *delegate.ivars().state_controls.borrow_mut() = controls_map;
    update_tab_prefixes(delegate, TAB_GENERAL);

    window
}

/// 侧栏:顶部 tab(General + 6 状态,左对齐;状态 tab = 当前色圆点 + 英文简称)+ 底部单色图标行。
fn build_sidebar(sidebar: &Retained<NSView>, delegate: &AppDelegate) {
    let tab_w = SIDEBAR_W - 16.0;
    // General
    add_tab_button(
        sidebar,
        NSRect::new(NSPoint::new(8.0, H - 44.0), NSSize::new(tab_w, 28.0)),
        "General",
        None,
        TAB_GENERAL,
        delegate,
    );
    // 状态 tab(自上而下)
    for (i, (tag, key, name)) in STATE_TABS.iter().enumerate() {
        let y = H - 44.0 - (i as CGFloat + 1.0) * 32.0;
        let color = delegate.ivars().settings.borrow().style_for(*key).color;
        let img = swatch_image(color, 14.0, false);
        add_tab_button(
            sidebar,
            NSRect::new(NSPoint::new(8.0, y), NSSize::new(tab_w, 28.0)),
            name,
            Some(&img),
            *tag,
            delegate,
        );
    }
    // 底部单色 SF Symbol 图标行(L→R:关于 functional / 其余占位禁用)
    let icons: [(&str, i64, bool); 5] = [
        ("info.circle", TAB_ABOUT, true),
        ("globe", 8, false),
        ("ant", 9, false),
        ("heart", 10, false),
        ("power", 11, false),
    ];
    let icon_w = (SIDEBAR_W - 16.0) / icons.len() as CGFloat;
    for (i, (sym, tag, enabled)) in icons.iter().enumerate() {
        let x = 8.0 + i as CGFloat * icon_w;
        let btn = add_icon_button(
            sidebar,
            NSRect::new(NSPoint::new(x, 8.0), NSSize::new(icon_w, 28.0)),
            sym,
            *tag,
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
    let mut tabs: Vec<(i64, &str)> = vec![(TAB_GENERAL, "General")];
    tabs.extend(STATE_TABS.iter().map(|(t, _, n)| (*t, *n)));
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

/// content view 里按 tag 找子视图(仅用于侧栏 tab 按钮;控件用 StateControls)。
pub fn view_with_tag(view: &Retained<NSView>, tag: i64) -> Option<Retained<NSView>> {
    unsafe { msg_send![view, viewWithTag: tag] }
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
        "General",
        false,
        true,
    );
    y -= 38.0;

    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(160.0, 20.0)),
        "Light size",
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

    let click_on = *delegate.ivars().click_through.borrow();
    add_checkbox(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        "Click-through(取消则可用鼠标拖动)",
        click_on,
        sel!(toggleClickThrough:),
        delegate,
    );
    y -= 36.0;

    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, y), NSSize::new(120.0, 20.0)),
        "Poll interval",
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

    let launch = add_checkbox(
        &pane,
        NSRect::new(
            NSPoint::new(pad, y),
            NSSize::new(CONTENT_W - pad * 2.0, 22.0),
        ),
        "Launch at login(待实现)",
        false,
        sel!(noop:),
        delegate,
    );
    unsafe {
        let _: () = msg_send![&launch, setEnabled: Bool::NO];
    }

    pane
}

fn build_state_pane(delegate: &AppDelegate, key: StyleKey) -> (Retained<NSView>, StateControls) {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let pad = 24.0;
    let base = tab_of_key(key) * 1000;
    let name = STATE_TABS
        .iter()
        .find(|(_, k, _)| *k == key)
        .map(|(_, _, n)| *n)
        .unwrap_or("?");

    // 标题 + 右上角 Reset
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, H - 48.0), NSSize::new(200.0, 22.0)),
        name,
        false,
        true,
    );
    let reset = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(CONTENT_W - pad - 70.0, H - 48.0),
            NSSize::new(70.0, 24.0),
        ),
        "Reset",
        base + RESET_OFF,
        sel!(resetStateStyle:),
        delegate,
    );
    let _ = reset;

    // Color:横向 6 色块单选(选中带环)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, H - 96.0), NSSize::new(120.0, 20.0)),
        "Color",
        false,
        false,
    );
    let mut color_btns: Vec<Retained<NSButton>> = Vec::with_capacity(6);
    for (i, &color) in COLOR_ORDER.iter().enumerate() {
        let x = pad + i as CGFloat * 40.0;
        color_btns.push(add_swatch_button(
            &pane,
            NSRect::new(NSPoint::new(x, H - 128.0), NSSize::new(SWATCH_D, SWATCH_D)),
            color,
            base + COLOR_OFF + i as i64,
            delegate,
        ));
    }

    // Animation:3 单选(radio)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, H - 168.0), NSSize::new(120.0, 20.0)),
        "Animation",
        false,
        false,
    );
    let anim_names = ["Steady", "Pulse", "Ripple"];
    let mut anim_btns: Vec<Retained<NSButton>> = Vec::with_capacity(3);
    for (i, nm) in anim_names.iter().enumerate() {
        anim_btns.push(add_radio_button(
            &pane,
            NSRect::new(
                NSPoint::new(pad + i as CGFloat * 100.0, H - 200.0),
                NSSize::new(90.0, 22.0),
            ),
            nm,
            base + ANIM_OFF + i as i64,
            delegate,
        ));
    }

    // Speed:Hz 滑块(0.2–5)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(pad, H - 244.0), NSSize::new(120.0, 20.0)),
        "Speed",
        false,
        false,
    );
    let speed = add_slider(
        &pane,
        NSRect::new(NSPoint::new(pad, H - 276.0), NSSize::new(240.0, 22.0)),
        SPEED_MIN,
        SPEED_MAX,
        1.0,
        sel!(changeSpeed:),
        delegate,
    );
    set_tag(&speed, base + SPEED_OFF);
    let speed_label = add_text(
        &pane,
        NSRect::new(
            NSPoint::new(pad + 250.0, H - 276.0),
            NSSize::new(70.0, 20.0),
        ),
        "—",
        false,
        false,
    );
    set_tag(&speed_label, base + SPEED_LABEL_OFF);

    let controls = StateControls {
        color: color_btns,
        anim: anim_btns,
        speed,
        speed_label,
    };
    // 按 settings 初始刷新一遍(选中态 / radio / 速度)
    let style = delegate.ivars().settings.borrow().style_for(key);
    refresh_state_controls(&controls, style);
    (pane, controls)
}

/// 按某状态的当前样式,刷新其 pane 的色块(选中带环)/ radio 选中 / 速度滑块+标签。
pub fn refresh_state_controls(c: &StateControls, style: StateStyle) {
    let steady = style.anim == Anim::Steady;
    for (i, btn) in c.color.iter().enumerate() {
        let img = swatch_image(COLOR_ORDER[i], SWATCH_D, style.color == COLOR_ORDER[i]);
        unsafe {
            let _: () = msg_send![btn, setImage: &*img];
        }
    }
    for (i, btn) in c.anim.iter().enumerate() {
        let on = style.anim == ANIM_ORDER[i];
        unsafe {
            let _: () = msg_send![btn, setState: if on { 1i64 } else { 0 }];
        }
    }
    let hz = if steady {
        1.0
    } else {
        hz_of(style.period_ms).clamp(SPEED_MIN, SPEED_MAX)
    };
    let text = if steady {
        "—".to_string()
    } else {
        format!("{:.1} Hz", hz)
    };
    unsafe {
        let _: () = msg_send![&c.speed, setEnabled: Bool::new(!steady)];
        let _: () = msg_send![&c.speed, setDoubleValue: hz];
        let _: () = msg_send![&c.speed_label, setStringValue: &*NSString::from_str(&text)];
    }
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
        &format!("Version {}", env!("CARGO_PKG_VERSION")),
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

/// 无边框按钮(Reset 等):标题 + action。
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

/// 侧栏 tab 按钮:无边框、左对齐;可选 image(状态色圆点)置于标题左侧。
fn add_tab_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    image: Option<&Retained<NSImage>>,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setBordered: Bool::NO];
        let _: () = msg_send![&btn, setAlignment: 0i64]; // NSTextAlignmentLeft
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str(title)];
        if let Some(img) = image {
            let _: () = msg_send![&btn, setImage: &**img];
            let _: () = msg_send![&btn, setImagePosition: 2i64]; // NSCellImagePositionImageLeft
        }
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(switchSettingsTab:)];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

/// 底栏图标按钮:单色 SF Symbol 图标,点击 switchSettingsTab:。
fn add_icon_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    symbol: &str,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    let img = sf_symbol(symbol);
    unsafe {
        let _: () = msg_send![&btn, setBordered: Bool::NO];
        let _: () = msg_send![&btn, setImage: &*img];
        let _: () = msg_send![&btn, setImagePosition: 5i64]; // NSCellImagePositionImageOnly
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(switchSettingsTab:)];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

/// 色块单选按钮:无边框,图片=该色 swatch。
fn add_swatch_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    color: Color,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    let img = swatch_image(color, SWATCH_D, false);
    unsafe {
        let _: () = msg_send![&btn, setBordered: Bool::NO];
        let _: () = msg_send![&btn, setButtonType: 4u64]; // NSButtonTypeRadio(让 AppKit 互斥)
        let _: () = msg_send![&btn, setImage: &*img];
        let _: () = msg_send![&btn, setImagePosition: 5i64]; // image only
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(changeColor:)];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

/// 动画单选:radio 按钮(标题=动画英文名)。
fn add_radio_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setButtonType: 4u64]; // NSButtonTypeRadio
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str(title)];
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(changeAnim:)];
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
            let _: () = msg_send![&label, setAlignment: 2i64]; // center
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
