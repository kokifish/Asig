//! 设置窗口(左侧栏导航)。界面文案按 `Settings.lang`(默认中文)本地化,可切全英文。
//! 左栏:General + 6 状态 tab(左对齐;状态 tab = 当前色圆点 + 本地化简称)+ 底部单色 SF Symbol
//! 图标行(关于 functional;其余占位禁用)。右区:8 pane。
//! 状态 pane = State Settings Card(Reset + Color 色块单选 + Animation 单选 + Speed Hz),
//! 颜色/动画/速度各占一行。

use std::collections::HashMap;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, Sel};
use objc2::{DefinedClass, MainThreadMarker, class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBox, NSButton, NSColor, NSFont, NSImage, NSPopUpButton, NSSlider,
    NSSplitViewController, NSSplitViewItem, NSSwitch, NSTextField, NSView, NSViewController,
    NSVisualEffectView, NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{Anim, Color, Lang, StateStyle, StyleKey};

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

const W: CGFloat = 680.0;
const H: CGFloat = 460.0;
const SIDEBAR_W: CGFloat = 170.0;
const CONTENT_W: CGFloat = W - SIDEBAR_W;
/// 标题栏高度。窗口用 fullSizeContentView + 透明标题栏(侧栏毛玻璃渗透到顶),故 sidebar/
/// content 两个 pane 都铺满整窗高度、顶部延展到标题栏下。但 pane 内的「内容」(tab / 标题 / 卡片)
/// 必须从标题栏下方开始,否则会被压在标题栏下/与红黄绿重叠。所有「距顶」锚点都扣除本值。
const TOP_INSET: CGFloat = 28.0;

/// 关于页显示的仓库链接(占位,改成真实仓库)。
const GITHUB_URL: &str = "https://github.com/koki/Asig";

pub const ANIM_ORDER: [Anim; 3] = [Anim::Steady, Anim::Pulse, Anim::Ripple];
pub const COLOR_ORDER: [Color; 6] = [
    Color::Green,
    Color::LightBlue,
    Color::Yellow,
    Color::Amber,
    Color::Red,
    Color::Purple,
];
/// 轮询间隔下拉的可选项(ms)。index ↔ 选中项。
pub const POLL_PRESETS_MS: [u32; 5] = [1000, 2000, 3000, 5000, 10000];

pub const TAB_GENERAL: i64 = 0;
pub const TAB_ABOUT: i64 = 7;

/// 状态 tab 顺序(DEV.md「Left Side Tabs」)。label 由 `Strings.state` 按本地化填。
const STATE_KEYS: [(i64, StyleKey); 6] = [
    (1, StyleKey::DoneNotif),
    (2, StyleKey::Done),
    (3, StyleKey::Working),
    (4, StyleKey::NeedsDeci),
    (5, StyleKey::Error),
    (6, StyleKey::Offline),
];

// 状态控件 tag sub 偏移(base = tab_id*1000)。
pub const COLOR_OFF: i64 = 10;
pub const ANIM_OFF: i64 = 20;
pub const SPEED_OFF: i64 = 30;
pub const SPEED_LABEL_OFF: i64 = 31;
pub const RESET_OFF: i64 = 40;
// General pane 语言单选 tag。
pub const LANG_EN_TAG: i64 = 501;
pub const LANG_ZH_TAG: i64 = 502;

pub const SPEED_MIN: f64 = 0.2;
pub const SPEED_MAX: f64 = 5.0;
const SWATCH_D: CGFloat = 24.0;

// 居中列布局(stats.app 风):内容居中成一列,分组圆角卡片,行 = 左标签 + 右控件。
const COL_W: CGFloat = 380.0;
const ROW_H: CGFloat = 32.0;

/// 卡片 frame:顶部 `top`、`rows` 行高。
fn card_frame(x0: CGFloat, top: CGFloat, rows: usize) -> NSRect {
    let h = rows as CGFloat * ROW_H + 16.0;
    NSRect::new(NSPoint::new(x0, top - h), NSSize::new(COL_W, h))
}

/// 第 i 行(0=最上)的标签/控件基线 y。
fn row_y(top: CGFloat, i: usize) -> CGFloat {
    top - 14.0 - i as CGFloat * ROW_H
}

/// 分组圆角卡片背景(NSBox custom:细边 + 圆角 + 浅填充),置于行后面。
fn add_card(pane: &Retained<NSView>, frame: NSRect) {
    let b: Retained<NSBox> = unsafe { msg_send![class!(NSBox), new] };
    unsafe {
        let _: () = msg_send![&b, setBoxType: 4u64]; // NSBoxCustom
        let _: () = msg_send![&b, setCornerRadius: 10.0f64];
        let _: () = msg_send![&b, setBorderWidth: 1.0f64];
        let border: Retained<NSColor> = msg_send![class!(NSColor), separatorColor];
        let _: () = msg_send![&b, setBorderColor: &*border];
        let fill: Retained<NSColor> = msg_send![class!(NSColor), controlBackgroundColor];
        let _: () = msg_send![&b, setFillColor: &*fill];
        let _: () = msg_send![&b, setTitle: &*NSString::from_str("")];
        let _: () = msg_send![&b, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*b];
    }
}

/// NSVisualEffectView 容器(material 原生材质:12=windowBackground / 18=contentBackground /
/// 7=sidebar)。blending=withinWindow、state=active。
fn effect_view(frame: NSRect, material: i64) -> Retained<NSVisualEffectView> {
    let alloc: Allocated<NSVisualEffectView> =
        unsafe { msg_send![class!(NSVisualEffectView), alloc] };
    let v: Retained<NSVisualEffectView> = unsafe { msg_send![alloc, initWithFrame: frame] };
    unsafe {
        let _: () = msg_send![&v, setMaterial: material];
        let _: () = msg_send![&v, setBlendingMode: 1i64]; // withinWindow
        let _: () = msg_send![&v, setState: 1i64]; // active
        let _: () = msg_send![&v, setWantsLayer: Bool::YES];
    }
    v
}

/// 一个状态 pane 的全部控件(类型化引用,便于 reset / 选择变更时批量刷新)。
pub struct StateControls {
    pub color: Vec<Retained<NSButton>>,
    pub anim: Vec<Retained<NSButton>>,
    pub speed: Retained<NSSlider>,
    pub speed_label: Retained<NSTextField>,
}

/// 当前语言的全部界面文案。
struct Strings {
    general: &'static str,
    light_size: &'static str,
    click_through: &'static str,
    poll_interval: &'static str,
    launch_login: &'static str,
    language: &'static str,
    reset: &'static str,
    color: &'static str,
    animation: &'static str,
    speed: &'static str,
    version: &'static str,
    state: [&'static str; 6], // 与 STATE_KEYS 同序
    anim: [&'static str; 3],
    poll_opts: [&'static str; 5],
    reset_confirm_title: &'static str,
    reset_confirm_msg: &'static str,
    reset_yes: &'static str,
    reset_no: &'static str,
}

/// General「Reset 全部」确认对话框的文案(按当前语言):(title, msg, yes, no)。
pub fn reset_confirm_texts(l: Lang) -> (&'static str, &'static str, &'static str, &'static str) {
    let s = strings_for(l);
    (
        s.reset_confirm_title,
        s.reset_confirm_msg,
        s.reset_yes,
        s.reset_no,
    )
}

fn strings_for(l: Lang) -> Strings {
    match l {
        Lang::Zh => Strings {
            general: "常规",
            light_size: "浮窗大小",
            click_through: "浮窗点击穿透(取消则可拖动)",
            poll_interval: "轮询间隔",
            launch_login: "开机启动(待实现)",
            language: "语言",
            reset: "重置",
            color: "颜色",
            animation: "效果",
            speed: "速度",
            version: "版本 ",
            state: ["完成通知", "完成", "运行", "待决策", "报错", "离线"],
            anim: ["常亮", "呼吸", "波纹"],
            poll_opts: ["1 秒", "2 秒", "3 秒", "5 秒", "10 秒"],
            reset_confirm_title: "重置全部设置",
            reset_confirm_msg: "将所有自定义(语言 + 各状态灯效)恢复为默认值。确认?",
            reset_yes: "重置",
            reset_no: "取消",
        },
        Lang::En => Strings {
            general: "General",
            light_size: "Light size",
            click_through: "Click-through (off = draggable)",
            poll_interval: "Poll interval",
            launch_login: "Launch at login (TBD)",
            language: "Language",
            reset: "Reset",
            color: "Color",
            animation: "Animation",
            speed: "Speed",
            version: "Version ",
            state: [
                "DoneNotif",
                "Done",
                "Working",
                "NeedsDeci",
                "Error",
                "Offline",
            ],
            anim: ["Steady", "Pulse", "Ripple"],
            poll_opts: ["1 s", "2 s", "3 s", "5 s", "10 s"],
            reset_confirm_title: "Reset all settings",
            reset_confirm_msg: "Restore all custom settings (language + per-state styles) to defaults?",
            reset_yes: "Reset",
            reset_no: "Cancel",
        },
    }
}

pub fn stylekey_of_tab(tab: i64) -> Option<StyleKey> {
    STATE_KEYS.iter().find(|(t, _)| *t == tab).map(|(_, k)| *k)
}

fn tab_of_key(key: StyleKey) -> i64 {
    STATE_KEYS
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(t, _)| *t)
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
    let mtm = MainThreadMarker::new().expect("settings build 须在主线程");
    let lang = delegate.ivars().settings.borrow().lang;
    let st = strings_for(lang);

    // 窗口:titled(1)|closable(2)|miniaturizable(4)|resizable(8)|fullSizeContentView(32768)
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: 32783u64,
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let _: () = msg_send![&window, setTitle: &*NSString::from_str("Asig")];
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
        let _: () = msg_send![&window, setOpaque: Bool::NO]; // 透明底,让 vibrancy 能模糊桌面
        let clear: Retained<NSColor> = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![&window, setBackgroundColor: &*clear];
        let _: () = msg_send![&window, setTitlebarAppearsTransparent: Bool::YES]; // 内容贯穿标题栏
        let _: () = msg_send![&window, setTitleVisibility: 1i64]; // hidden
        let _: () = msg_send![&window, setMovable: true];
        let _: () = msg_send![&window, setMinSize: NSSize::new(W, H)];
    }

    // 侧栏视图(透明;sidebarWithViewController 会套原生 sidebar 材质/vibrancy)。
    let sidebar = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(SIDEBAR_W, H),
    ));
    build_sidebar(&sidebar, delegate, &st);

    // 右区:普通 NSView 容器 + windowBackground 材质铺底;其上卡片(controlBackgroundColor)更亮。
    let content_area = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    {
        let bg = effect_view(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(CONTENT_W, H)),
            12, // windowBackground
        );
        unsafe {
            let _: () = msg_send![&bg, setAutoresizingMask: 18u64]; // width+height sizable
            let _: () = msg_send![&content_area, addSubview: &*bg]; // 铺底,pane 在其上
        }
    }
    // 8 pane:General + 6 状态(各带 StateControls)+ About。按 pane id(=索引)排。
    let mut panes: Vec<Retained<NSView>> = Vec::with_capacity(8);
    let mut controls_map: HashMap<StyleKey, StateControls> = HashMap::new();
    panes.push(build_general_pane(delegate, &st));
    for (i, (_, key)) in STATE_KEYS.iter().enumerate() {
        let (pane, c) = build_state_pane(delegate, *key, st.state[i], &st);
        controls_map.insert(*key, c);
        panes.push(pane);
    }
    panes.push(build_about_pane(&st));
    for (i, pane) in panes.iter().enumerate() {
        unsafe {
            let _: () = msg_send![pane, setHidden: Bool::new(i != 0)];
            let _: () = msg_send![&content_area, addSubview: &**pane];
        }
    }

    // 方案 A:包成 NSViewController → NSSplitViewController(sidebarWithViewController 给原生侧栏)。
    let sidebar_vc = NSViewController::new(mtm);
    let content_vc = NSViewController::new(mtm);
    unsafe {
        let _: () = msg_send![&sidebar_vc, setView: &*sidebar];
        let _: () = msg_send![&content_vc, setView: &*content_area];
    }
    let sidebar_item = NSSplitViewItem::sidebarWithViewController(&sidebar_vc);
    let item_alloc: Allocated<NSSplitViewItem> =
        unsafe { msg_send![class!(NSSplitViewItem), alloc] };
    let content_item = NSSplitViewItem::init(item_alloc);
    unsafe {
        let _: () = msg_send![&content_item, setViewController: &*content_vc];
        for it in [&sidebar_item, &content_item] {
            let _: () = msg_send![&**it, setCanCollapse: false];
        }
        let _: () = msg_send![&sidebar_item, setMinimumThickness: SIDEBAR_W];
        let _: () = msg_send![&content_item, setMinimumThickness: CONTENT_W];
    }
    let svc = NSSplitViewController::new(mtm);
    svc.addSplitViewItem(&sidebar_item);
    svc.addSplitViewItem(&content_item);
    unsafe {
        let _: () = msg_send![&window, setContentViewController: Some(&*svc)];
    }

    *delegate.ivars().settings_sidebar.borrow_mut() = Some(sidebar);
    *delegate.ivars().settings_content.borrow_mut() = Some(content_area);
    *delegate.ivars().settings_panes.borrow_mut() = Some(panes);
    *delegate.ivars().settings_selected.borrow_mut() = TAB_GENERAL;
    *delegate.ivars().state_controls.borrow_mut() = controls_map;
    update_tab_prefixes(delegate, TAB_GENERAL);

    // ASIG_TAB(dev):直接打开指定 pane(1..7),便于逐页截图;默认 0(常规)。
    if let Some(n) = std::env::var("ASIG_TAB").ok().and_then(|s| s.parse::<i64>().ok()) {
        if (1..8).contains(&n) {
            {
                let panes_ref = delegate.ivars().settings_panes.borrow();
                if let Some(v) = panes_ref.as_ref() {
                    unsafe {
                        if let Some(p0) = v.get(0) {
                            let _: () = msg_send![p0, setHidden: Bool::YES];
                        }
                        if let Some(pn) = v.get(n as usize) {
                            let _: () = msg_send![pn, setHidden: Bool::NO];
                        }
                    }
                }
            }
            *delegate.ivars().settings_selected.borrow_mut() = n;
            update_tab_prefixes(delegate, n);
        }
    }

    window
}

/// 侧栏:顶部 tab(General + 6 状态,左对齐;状态 tab = 当前色圆点 + 本地化简称)+ 底部单色图标行。
fn build_sidebar(sidebar: &Retained<NSView>, delegate: &AppDelegate, st: &Strings) {
    let tab_w = SIDEBAR_W - 16.0;
    add_tab_button(
        sidebar,
        NSRect::new(
            NSPoint::new(8.0, H - 52.0 - TOP_INSET),
            NSSize::new(tab_w, 28.0),
        ),
        st.general,
        None,
        TAB_GENERAL,
        delegate,
    );
    for (i, (tag, key)) in STATE_KEYS.iter().enumerate() {
        let y = H - 52.0 - TOP_INSET - (i as CGFloat + 1.0) * 32.0;
        let color = delegate.ivars().settings.borrow().style_for(*key).color;
        let img = swatch_image(color, 14.0, false);
        add_tab_button(
            sidebar,
            NSRect::new(NSPoint::new(8.0, y), NSSize::new(tab_w, 28.0)),
            st.state[i],
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
    let st = strings_for(delegate.ivars().settings.borrow().lang);
    let mut labels: Vec<(i64, &str)> = vec![(TAB_GENERAL, st.general)];
    labels.extend(
        STATE_KEYS
            .iter()
            .zip(st.state.iter())
            .map(|((t, _), n)| (*t, *n)),
    );
    for (tag, label) in labels {
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

/// content view 里按 tag 找子视图(仅侧栏 tab 按钮;状态控件用 StateControls)。
pub fn view_with_tag(view: &Retained<NSView>, tag: i64) -> Option<Retained<NSView>> {
    unsafe { msg_send![view, viewWithTag: tag] }
}

// ---- 各 pane ----

fn build_general_pane(delegate: &AppDelegate, st: &Strings) -> Retained<NSView> {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let x0 = (CONTENT_W - COL_W) / 2.0;
    let lx = x0 + 16.0; // 标签 x
    let cx = x0 + 140.0; // 控件 x
    let cw = COL_W - 140.0 - 16.0; // 控件区宽
    let mut y = H - 48.0 - TOP_INSET;

    // 标题(居中)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, y), NSSize::new(CONTENT_W, 24.0)),
        st.general,
        true,
        true,
    );
    y -= 32.0;

    // —— Card:常规设置(4 行)——
    add_card(&pane, card_frame(x0, y, 4));
    // Light size
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 0)), NSSize::new(120.0, 20.0)),
        st.light_size,
        false,
        false,
    );
    let dot = delegate.ivars().settings.borrow().dot_size as f64;
    add_slider(
        &pane,
        NSRect::new(NSPoint::new(cx, row_y(y, 0) - 2.0), NSSize::new(cw, 22.0)),
        8.0,
        40.0,
        dot,
        sel!(changeSize:),
        delegate,
    );
    // Click-through(标签 + 开关)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 1)), NSSize::new(120.0, 20.0)),
        st.click_through,
        false,
        false,
    );
    add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx + cw - 40.0, row_y(y, 1) - 1.0),
            NSSize::new(40.0, 22.0),
        ),
        *delegate.ivars().click_through.borrow(),
        sel!(toggleClickThrough:),
        delegate,
    );
    // Poll interval
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 2)), NSSize::new(120.0, 20.0)),
        st.poll_interval,
        false,
        false,
    );
    let poll_ms = delegate.ivars().settings.borrow().poll_interval_ms;
    add_popup(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_y(y, 2) - 4.0),
            NSSize::new(120.0, 26.0),
        ),
        &st.poll_opts,
        poll_preset_index(poll_ms),
        sel!(changePollInterval:),
        delegate,
        0,
    );
    // Launch at login(标签 + 开关,占位禁用)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 3)), NSSize::new(120.0, 20.0)),
        st.launch_login,
        false,
        false,
    );
    let launch = add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx + cw - 40.0, row_y(y, 3) - 1.0),
            NSSize::new(40.0, 22.0),
        ),
        false,
        sel!(noop:),
        delegate,
    );
    unsafe {
        let _: () = msg_send![&launch, setEnabled: Bool::NO];
    }
    y -= 4.0 * ROW_H + 16.0 + 20.0;

    // —— Card:语言(1 行)——
    add_card(&pane, card_frame(x0, y, 1));
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 0)), NSSize::new(80.0, 20.0)),
        st.language,
        false,
        false,
    );
    let lang = delegate.ivars().settings.borrow().lang;
    add_radio_button(
        &pane,
        NSRect::new(NSPoint::new(cx, row_y(y, 0)), NSSize::new(90.0, 22.0)),
        "English",
        LANG_EN_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    add_radio_button(
        &pane,
        NSRect::new(
            NSPoint::new(cx + 100.0, row_y(y, 0)),
            NSSize::new(90.0, 22.0),
        ),
        "中文",
        LANG_ZH_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    let want_tag = if lang == Lang::En {
        LANG_EN_TAG
    } else {
        LANG_ZH_TAG
    };
    for t in [LANG_EN_TAG, LANG_ZH_TAG] {
        if let Some(b) = view_with_tag(&pane, t) {
            unsafe {
                let _: () = msg_send![&b, setState: if t == want_tag { 1i64 } else { 0 }];
            }
        }
    }
    y -= 1.0 * ROW_H + 16.0 + 24.0;

    // Reset(全部,居中)
    let _ = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new((CONTENT_W - 120.0) / 2.0, y),
            NSSize::new(120.0, 28.0),
        ),
        st.reset,
        0,
        sel!(resetAll:),
        delegate,
    );

    pane
}

fn build_state_pane(
    delegate: &AppDelegate,
    key: StyleKey,
    name: &str,
    st: &Strings,
) -> (Retained<NSView>, StateControls) {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let x0 = (CONTENT_W - COL_W) / 2.0;
    let lx = x0 + 16.0;
    let cx = x0 + 140.0;
    let cw = COL_W - 140.0 - 16.0;
    let base = tab_of_key(key) * 1000;
    let mut y = H - 48.0 - TOP_INSET;

    // 标题(居中)+ 右上角 Reset
    add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, y), NSSize::new(CONTENT_W, 24.0)),
        name,
        true,
        true,
    );
    let _ = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(CONTENT_W - x0 - 70.0, y),
            NSSize::new(70.0, 24.0),
        ),
        st.reset,
        base + RESET_OFF,
        sel!(resetStateStyle:),
        delegate,
    );
    y -= 32.0;

    // —— Card:状态(3 行)——
    add_card(&pane, card_frame(x0, y, 3));
    // Color(标签 + 横向色块)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 0)), NSSize::new(120.0, 20.0)),
        st.color,
        false,
        false,
    );
    let mut color_btns: Vec<Retained<NSButton>> = Vec::with_capacity(6);
    for (i, &color) in COLOR_ORDER.iter().enumerate() {
        let sx = cx + i as CGFloat * 32.0;
        color_btns.push(add_swatch_button(
            &pane,
            NSRect::new(
                NSPoint::new(sx, row_y(y, 0) - 4.0),
                NSSize::new(SWATCH_D, SWATCH_D),
            ),
            color,
            base + COLOR_OFF + i as i64,
            delegate,
        ));
    }
    // Animation(标签 + 3 单选)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 1)), NSSize::new(120.0, 20.0)),
        st.animation,
        false,
        false,
    );
    let mut anim_btns: Vec<Retained<NSButton>> = Vec::with_capacity(3);
    for (i, &nm) in st.anim.iter().enumerate() {
        anim_btns.push(add_radio_button(
            &pane,
            NSRect::new(
                NSPoint::new(cx + i as CGFloat * 76.0, row_y(y, 1)),
                NSSize::new(72.0, 22.0),
            ),
            nm,
            base + ANIM_OFF + i as i64,
            delegate,
            sel!(changeAnim:),
        ));
    }
    // Speed(标签 + 滑块 + Hz)
    add_text(
        &pane,
        NSRect::new(NSPoint::new(lx, row_y(y, 2)), NSSize::new(120.0, 20.0)),
        st.speed,
        false,
        false,
    );
    let speed = add_slider(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_y(y, 2) - 2.0),
            NSSize::new(cw - 64.0, 22.0),
        ),
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
            NSPoint::new(cx + cw - 56.0, row_y(y, 2) + 2.0),
            NSSize::new(56.0, 20.0),
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
    let style = delegate.ivars().settings.borrow().style_for(key);
    refresh_state_controls(&controls, style);
    (pane, controls)
}

/// 按某状态当前样式,刷新其 pane 的色块(选中带环)/ radio 选中 / 速度滑块+标签。
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

fn build_about_pane(st: &Strings) -> Retained<NSView> {
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
        &format!("{}{}", st.version, env!("CARGO_PKG_VERSION")),
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

/// 无边框按钮(Reset):标题 + action。
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
        let _: () = msg_send![&btn, setBezelStyle: 1u64]; // rounded
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
            let _: () = msg_send![&btn, setImagePosition: 2i64]; // image left
        }
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(switchSettingsTab:)];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

/// 底栏图标按钮:单色 SF Symbol,无标题(image only)。
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
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str("")]; // 消掉默认 "Button"
        let _: () = msg_send![&btn, setImage: &*img];
        let _: () = msg_send![&btn, setImagePosition: 5i64]; // image only
        let _: () = msg_send![&btn, setTag: tag];
        let _: () = msg_send![&btn, setTarget: delegate];
        let _: () = msg_send![&btn, setAction: sel!(switchSettingsTab:)];
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*btn];
    }
    btn
}

/// 色块单选按钮:无边框、无标题,图片=该色 swatch(选中带环)。
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
        let _: () = msg_send![&btn, setTitle: &*NSString::from_str("")]; // 消掉默认 "Button"
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

/// 单选按钮(radio):标题 + action。
fn add_radio_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    tag: i64,
    delegate: &AppDelegate,
    action: Sel,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe { msg_send![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn, setButtonType: 4u64]; // NSButtonTypeRadio
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
            let _: () = msg_send![&label, setAlignment: 2i64];
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

/// NSSwitch(现代滑动开关,原生)。用于「点击穿透 / 开机启动」等开关行。
fn add_switch(
    pane: &Retained<NSView>,
    frame: NSRect,
    on: bool,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSSwitch> {
    let mtm = MainThreadMarker::new().expect("NSSwitch 须主线程");
    let sw = NSSwitch::new(mtm);
    unsafe {
        let _: () = msg_send![&sw, setState: if on { 1i64 } else { 0 }];
        let _: () = msg_send![&sw, setTarget: delegate];
        let _: () = msg_send![&sw, setAction: action];
        let _: () = msg_send![&sw, setFrame: frame];
        let _: () = msg_send![&**pane, addSubview: &*sw];
    }
    sw
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
