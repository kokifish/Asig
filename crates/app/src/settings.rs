//! 设置窗口(左侧栏导航)。界面文案按 `Settings.lang`(默认中文)本地化,可切全英文。
//! 左栏:General + 6 状态 tab(左对齐;状态 tab = 当前色圆点 + 本地化简称)+ 底部单色 SF Symbol
//! 图标行(关于 functional;其余占位禁用)。右区:8 pane。
//! 状态 pane = State Settings Card(Reset + Color 色块单选 + Animation 单选 + Speed Hz),
//! 颜色/动画/速度各占一行。

use std::collections::HashMap;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, Bool, NSObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBox, NSButton, NSColor, NSFont, NSImage, NSPopUpButton, NSSlider, NSSwitch,
    NSTextField, NSView, NSWindow,
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
const CONTENT_PAD_X: CGFloat = 26.0;
const CONTENT_HEADER_H: CGFloat = 26.0;
/// 标题(下方不再有横线)到首张卡片的间距。
const HEADER_GAP: CGFloat = 16.0;
/// 标题栏高度。窗口 fullSizeContentView + 透明标题栏(主液态玻璃渗透到顶),但 pane 内的「内容」
/// (tab / 标题 / 卡片)必须从标题栏下方开始,否则会压在标题栏下/与红黄绿重叠。距顶锚点扣除本值。
const TOP_INSET: CGFloat = 28.0;
/// 浮动侧栏玻璃面板:距窗边留白(左/下/右),面板顶到标题栏下、底到窗底留白。
const SIDEBAR_INSET: CGFloat = 10.0;
const SIDEBAR_PANE_W: CGFloat = SIDEBAR_W - 2.0 * SIDEBAR_INSET;
const SIDEBAR_PANE_H: CGFloat = H - TOP_INSET - SIDEBAR_INSET;

/// 关于页显示的仓库链接(占位,改成真实仓库)。
const GITHUB_URL: &str = "https://github.com/koki/Asig";

pub const ANIM_ORDER: [Anim; 3] = [Anim::Steady, Anim::Pulse, Anim::Ripple];
pub const COLOR_ORDER: [Color; 6] = [
    Color::LightBlue,
    Color::Green,
    Color::Yellow,
    Color::Amber,
    Color::Red,
    Color::Purple,
];
/// 轮询间隔下拉的可选项(ms)。index ↔ 选中项。
pub const POLL_PRESETS_MS: [u32; 6] = [1000, 2000, 3000, 5000, 10000, 15000];

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
// General pane「浮窗灯大小」右侧 `xx px` 实时标签 tag(changeSize 时按它刷新)。
pub const SIZE_LABEL_TAG: i64 = 503;

pub const SPEED_MIN: f64 = 0.2;
pub const SPEED_MAX: f64 = 5.0;
const SWATCH_D: CGFloat = 24.0;

// 右区内容布局:标题属于 content panel 的 header;卡片与标题左边缘对齐。
const COL_W: CGFloat = CONTENT_W - CONTENT_PAD_X * 2.0;
const ROW_H: CGFloat = 32.0;
/// 卡片内顶部/底部留白;行间距 = ROW_H。所有行内容垂直居中对齐到 row_center_y。
const CARD_TOP_PAD: CGFloat = 10.0;
const CARD_BOT_PAD: CGFloat = 10.0;
/// 卡片之间的统一间距。
const CARD_GAP: CGFloat = 20.0;

/// `rows` 行卡片的总高度。
fn card_height(rows: usize) -> CGFloat {
    CARD_TOP_PAD + rows as CGFloat * ROW_H + CARD_BOT_PAD
}

/// 卡片 frame:顶部边在 `top`、`rows` 行高(含上下留白)。
fn card_frame(x0: CGFloat, top: CGFloat, rows: usize) -> NSRect {
    let h = card_height(rows);
    NSRect::new(NSPoint::new(x0, top - h), NSSize::new(COL_W, h))
}

/// 第 i 行(0=最上)的垂直中心 y。所有 label 与控件都对齐到它(居中制,杜绝错位)。
fn row_center_y(top: CGFloat, i: usize) -> CGFloat {
    top - CARD_TOP_PAD - (i as CGFloat + 0.5) * ROW_H
}

/// 分组圆角卡片背景(NSBox custom:细边 + 圆角 + 浅填充),置于行后面。
fn add_card(pane: &Retained<NSView>, frame: NSRect) {
    let b: Retained<NSBox> = unsafe { msg_send![class!(NSBox), new] };
    unsafe {
        let _: () = msg_send![&b, setBoxType: 4u64]; // NSBoxCustom
        let _: () = msg_send![&b, setCornerRadius: 10.0f64];
        let _: () = msg_send![&b, setBorderWidth: 0.0f64];
        let fill: Retained<NSColor> = msg_send![class!(NSColor), quaternaryLabelColor];
        let _: () = msg_send![&b, setFillColor: &*fill];
        let _: () = msg_send![&b, setTitle: &*NSString::from_str("")];
        let _: () = msg_send![&b, setFrame: frame];
        let _: () = msg_send![&b, setWantsLayer: Bool::YES];
        let layer: Retained<NSObject> = msg_send![&b, layer];
        let _: () = msg_send![&layer, setCornerCurve: &*NSString::from_str("continuous")];
        let _: () = msg_send![&**pane, addSubview: &*b];
    }
}

/// 运行时是否存在真·液态玻璃类(macOS 26+)。minos=11.0,旧系统无此类,须回退 vibrancy。
fn glass_available() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// 一块液态玻璃面板 + 它「承载 UI 的 content 视图」。两种后端、上层无感:UI 一律加到 `content`。
/// - macOS 26+:NSGlassEffectView,UI 必须放进其 contentView(Apple 文档明确要求;叠成兄弟视图
///   会被盖住 —— 这正是早先 NSGlassEffectView 失败的原因)。cornerRadius 决定玻璃形状圆角。
/// - 旧系统:NSVisualEffectView(`fallback_material`),UI 作子视图叠在 vibrancy 上(`content` 即其自身)。
///
/// 全程 msg_send! 构造并上转为 NSView,与既有 NSVisualEffectView 用法一致(绕开 Retained 上转)。
struct GlassPane {
    view: Retained<NSView>,
    content: Retained<NSView>,
}

fn glass_pane(frame: NSRect, corner_radius: CGFloat, fallback_material: i64) -> GlassPane {
    // Reduce Transparency 开启时跳过 NSGlassEffectView,改走 NSVisualEffectView 分支
    // (它在 Reduce Transparency 下自动变不透明实色),保证文字可读。
    if glass_available() && !crate::overlay::reduce_transparency_on() {
        let g: Retained<NSView> = unsafe { msg_send![class!(NSGlassEffectView), new] };
        let content = new_view(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        unsafe {
            let _: () = msg_send![&g, setFrame: frame];
            let _: () = msg_send![&g, setCornerRadius: corner_radius];
            let _: () = msg_send![&g, setContentView: Some(&*content)];
        }
        GlassPane { view: g, content }
    } else {
        let alloc: Allocated<NSView> = unsafe { msg_send![class!(NSVisualEffectView), alloc] };
        let v: Retained<NSView> = unsafe { msg_send![alloc, initWithFrame: frame] };
        unsafe {
            let _: () = msg_send![&v, setMaterial: fallback_material];
            let _: () = msg_send![&v, setBlendingMode: 0i64]; // behindWindow — 模糊窗口背后
            let _: () = msg_send![&v, setState: 1i64]; // active
            let _: () = msg_send![&v, setWantsLayer: Bool::YES];
        }
        GlassPane {
            view: v.clone(),
            content: v,
        }
    }
}

/// 侧栏选中药丸 = 实心强调色圆角块(controlAccentColor)。玻璃/vibrancy 材质的选中态在已带玻璃
/// 的侧栏上会与背景融为一体、不可辨(实测 NSGlassEffectView tint / NSVisualEffectView Selection
/// 均不可见),故用实心强调色(同 stats.app 的 selectedContentBackgroundColor),在玻璃侧栏上清晰、
/// 读作「选中」。一个共享视图,选中时移到对应 tab 行(见 update_selection)。初始隐藏。
fn make_selection_pill() -> Retained<NSView> {
    let b: Retained<NSView> = unsafe { msg_send![class!(NSBox), new] };
    unsafe {
        let _: () = msg_send![&b, setBoxType: 4u64]; // NSBoxCustom
        let _: () = msg_send![&b, setCornerRadius: 8.0f64];
        let _: () = msg_send![&b, setBorderWidth: 0.0f64];
        let accent: Retained<NSColor> = msg_send![class!(NSColor), controlAccentColor];
        let _: () = msg_send![&b, setFillColor: &*accent];
        let _: () = msg_send![&b, setTitle: &*NSString::from_str("")];
        let _: () = msg_send![&b, setWantsLayer: Bool::YES];
        let layer: Retained<NSObject> = msg_send![&b, layer];
        let _: () = msg_send![&layer, setCornerCurve: &*NSString::from_str("continuous")];
        let _: () = msg_send![&b, setHidden: Bool::YES]; // 初始隐藏,update_selection 时显示
    }
    b
}

/// 给 borderless tab 按钮设文字色:选中 = 白、否则 = labelColor。用 attributedTitle
/// 实现(borderless NSButton 默认标题色无法直接改)。状态色圆点图片保持彩色不变。
fn set_tab_title(button: &Retained<NSView>, label: &str, selected: bool) {
    let color: Retained<NSColor> = if selected {
        unsafe { msg_send![class!(NSColor), whiteColor] }
    } else {
        unsafe { msg_send![class!(NSColor), labelColor] }
    };
    unsafe {
        let attrs: Retained<NSObject> = msg_send![
            class!(NSDictionary),
            dictionaryWithObject: &*color,
            forKey: &*NSString::from_str("NSColor"), // NSForegroundColorAttributeName
        ];
        let astr: Allocated<NSObject> = msg_send![class!(NSAttributedString), alloc];
        let astr: Retained<NSObject> = msg_send![
            astr,
            initWithString: &*NSString::from_str(label),
            attributes: &*attrs,
        ];
        let _: () = msg_send![&**button, setAttributedTitle: &*astr];
    }
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
    about: &'static str,
    light_size: &'static str,
    click_through: &'static str,
    poll_interval: &'static str,
    launch_login: &'static str,
    language: &'static str,
    reset: &'static str,
    reset_all: &'static str,
    color: &'static str,
    animation: &'static str,
    speed: &'static str,
    version: &'static str,
    state: [&'static str; 6], // 与 STATE_KEYS 同序
    anim: [&'static str; 3],
    poll_opts: [&'static str; 6],
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
            general: "常规设置",
            about: "关于",
            light_size: "浮窗灯大小",
            click_through: "点击穿透(取消则可拖动)",
            poll_interval: "Agent状态轮询间隔",
            launch_login: "开机自启动(待实现)",
            language: "语言",
            reset: "重置",
            reset_all: "重置所有",
            color: "颜色",
            animation: "效果",
            speed: "速度",
            version: "版本 ",
            state: ["完成通知", "完成", "运行", "待决策", "报错", "离线"],
            anim: ["常亮", "呼吸", "波纹"],
            poll_opts: ["1 秒", "2 秒", "3 秒", "5 秒", "10 秒", "15 秒"],
            reset_confirm_title: "重置全部设置",
            reset_confirm_msg: "将所有自定义(语言 + 各状态灯效)恢复为默认值。确认?",
            reset_yes: "重置",
            reset_no: "取消",
        },
        Lang::En => Strings {
            general: "General Settings",
            about: "About",
            light_size: "Light size",
            click_through: "Click-through (off = draggable)",
            poll_interval: "Agent poll interval",
            launch_login: "Launch at login (TBD)",
            language: "Language",
            reset: "Reset",
            reset_all: "Reset All",
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
            poll_opts: ["1 s", "2 s", "3 s", "5 s", "10 s", "15 s"],
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
        let _: () = msg_send![&window, setTitlebarSeparatorStyle: 1i64]; // none — 玻璃贯穿标题栏,无顶部分隔线
        let _: () = msg_send![&window, setMovable: true];
        let _: () = msg_send![&window, setMinSize: NSSize::new(W, H)];
    }

    // 右区:透明 NSView,8 pane 叠在其上。origin 在 SIDEBAR_W,铺在主玻璃上(无外框)。
    let content_area = new_view(NSRect::new(
        NSPoint::new(SIDEBAR_W, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
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

    // 真·液态玻璃承载视图 root(普通 NSView;刻意不用 NSGlassEffectContainerView —— 它会把
    // 重叠/相邻的玻璃合并成一次模糊,令浮动侧栏失去层次)。root 内:主玻璃(满窗,承载右区内容)
    // + 浮动侧栏玻璃(左侧圆角,承载 tab/图标)两块独立玻璃叠放;侧栏因四周留白 + 二次模糊
    // 读作浮动玻璃面板,内容在主玻璃上无外框。
    let full = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let root = new_view(full);
    let main = glass_pane(full, 0.0, 12); // 主玻璃满窗(窗口自裁圆角);回退 material=WindowBackground
    let sidebar = glass_pane(
        NSRect::new(
            NSPoint::new(SIDEBAR_INSET, SIDEBAR_INSET),
            NSSize::new(SIDEBAR_PANE_W, SIDEBAR_PANE_H),
        ),
        14.0, // 浮动玻璃圆角
        7,    // 回退 material=Sidebar
    );
    // 侧栏 UI 建到浮动玻璃的 contentView 上。
    build_sidebar(&sidebar.content, delegate, &st);

    unsafe {
        let _: () = msg_send![&main.view, setAutoresizingMask: 18u64]; // 主玻璃随窗口缩放
        let _: () = msg_send![&root, addSubview: &*main.view]; // 主玻璃在底
        let _: () = msg_send![&*main.content, addSubview: &*content_area]; // 右区在主玻璃上
        let _: () = msg_send![&sidebar.view, setAutoresizingMask: 16u64]; // 侧栏固定宽,随高伸缩
        let _: () = msg_send![&root, addSubview: &*sidebar.view]; // 浮动侧栏在上
        let _: () = msg_send![&window, setContentView: &*root];
    }

    *delegate.ivars().settings_sidebar.borrow_mut() = Some(sidebar.content);
    *delegate.ivars().settings_content.borrow_mut() = Some(content_area);
    *delegate.ivars().settings_panes.borrow_mut() = Some(panes);
    *delegate.ivars().settings_selected.borrow_mut() = TAB_GENERAL;
    *delegate.ivars().state_controls.borrow_mut() = controls_map;
    update_selection(delegate, TAB_GENERAL);

    // ASIG_TAB(dev):直接打开指定 pane(1..7),便于逐页截图;默认 0(常规)。
    if let Some(n) = std::env::var("ASIG_TAB")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
    {
        if (1..8).contains(&n) {
            {
                let panes_ref = delegate.ivars().settings_panes.borrow();
                if let Some(v) = panes_ref.as_ref() {
                    unsafe {
                        if let Some(p0) = v.first() {
                            let _: () = msg_send![p0, setHidden: Bool::YES];
                        }
                        if let Some(pn) = v.get(n as usize) {
                            let _: () = msg_send![pn, setHidden: Bool::NO];
                        }
                    }
                }
            }
            *delegate.ivars().settings_selected.borrow_mut() = n;
            update_selection(delegate, n);
        }
    }

    window
}

/// 侧栏(建在浮动玻璃的 contentView 上):顶部 tab(General + 6 状态,左对齐;状态 tab =
/// 当前色圆点 + 本地化简称)+ 底部单色图标行。锚点按浮动面板自身尺寸(SIDEBAR_PANE_*)算。
fn build_sidebar(sidebar: &Retained<NSView>, delegate: &AppDelegate, st: &Strings) {
    // 选中药丸(实心强调色,共享):最先 addSubview → 落在所有 tab 按钮之下;update_selection
    // 时按选中按钮的 frame 移位并显示。状态色圆点保持彩色,仅文字随选中转白。
    let pill = make_selection_pill();
    unsafe {
        let _: () = msg_send![&**sidebar, addSubview: &*pill];
    }
    *delegate.ivars().settings_selection.borrow_mut() = Some(pill);

    let tab_w = SIDEBAR_PANE_W - 16.0;
    let top = SIDEBAR_PANE_H - 14.0 - 28.0; // 顶部留白 14 + tab 高 28
    // General tab = 齿轮(template SF Symbol)+ 常规设置;选中时 update_selection 把齿轮转白。
    let gear = sf_symbol("gearshape");
    unsafe {
        let _: () = msg_send![&gear, setTemplate: Bool::YES];
    }
    add_tab_button(
        sidebar,
        NSRect::new(NSPoint::new(8.0, top), NSSize::new(tab_w, 28.0)),
        st.general,
        Some(&gear),
        TAB_GENERAL,
        delegate,
    );
    for (i, (tag, key)) in STATE_KEYS.iter().enumerate() {
        let y = top - (i as CGFloat + 1.0) * 32.0;
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
    let icon_w = (SIDEBAR_PANE_W - 16.0) / icons.len() as CGFloat;
    for (i, (sym, tag, enabled)) in icons.iter().enumerate() {
        let x = 8.0 + i as CGFloat * icon_w;
        let btn = add_icon_button(
            sidebar,
            NSRect::new(NSPoint::new(x, 12.0), NSSize::new(icon_w, 28.0)),
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

/// 切换选中 tab:把液态玻璃药丸移到选中项并显示,选中文字转白、其余 labelColor。
pub fn update_selection(delegate: &AppDelegate, selected: i64) {
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
    let pill = delegate
        .ivars()
        .settings_selection
        .borrow()
        .as_ref()
        .cloned();
    let is_tab = labels.iter().any(|(t, _)| *t == selected);
    for (tag, label) in labels {
        let Some(b) = view_with_tag(&sidebar, tag) else {
            continue;
        };
        let is_sel = tag == selected;
        // 选中项:把药丸移到该按钮 frame 并显示。
        if is_sel {
            if let Some(p) = &pill {
                let frame: NSRect = unsafe { msg_send![&b, frame] };
                unsafe {
                    let _: () = msg_send![&**p, setFrame: frame];
                    let _: () = msg_send![&**p, setHidden: Bool::NO];
                }
            }
        }
        set_tab_title(&b, label, is_sel);
        // General tab 的齿轮(template)随选中转白;状态色点保持彩色不变。
        if tag == TAB_GENERAL {
            let tint: Retained<NSColor> = if is_sel {
                unsafe { msg_send![class!(NSColor), whiteColor] }
            } else {
                unsafe { msg_send![class!(NSColor), labelColor] }
            };
            unsafe {
                let _: () = msg_send![&b, setContentTintColor: &*tint];
            }
        }
    }
    // 选中的是非 tab 项(如「关于」= pane 7)时隐藏药丸 —— 不让某个 tab 仍读作选中。
    if !is_tab {
        if let Some(p) = &pill {
            unsafe {
                let _: () = msg_send![&**p, setHidden: Bool::YES];
            }
        }
    }
}

/// content view 里按 tag 找子视图(仅侧栏 tab 按钮;状态控件用 StateControls)。
pub fn view_with_tag(view: &Retained<NSView>, tag: i64) -> Option<Retained<NSView>> {
    unsafe { msg_send![view, viewWithTag: tag] }
}

// ---- 各 pane ----

/// header 图标:NSImageView + 单色(template)SF Symbol,contentTintColor=labelColor,随明暗。
fn add_header_icon(pane: &Retained<NSView>, frame: NSRect, symbol: &str) {
    let img = sf_symbol(symbol);
    unsafe {
        let _: () = msg_send![&img, setTemplate: Bool::YES];
        let iv: Retained<NSView> = msg_send![class!(NSImageView), new];
        let _: () = msg_send![&iv, setFrame: frame];
        let _: () = msg_send![&iv, setImage: &*img];
        let _: () = msg_send![&iv, setImageScaling: 0i64]; // scaleProportionallyDown
        let tint: Retained<NSColor> = msg_send![class!(NSColor), labelColor];
        let _: () = msg_send![&iv, setContentTintColor: &*tint];
        let _: () = msg_send![&**pane, addSubview: &*iv];
    }
}

fn build_general_pane(delegate: &AppDelegate, st: &Strings) -> Retained<NSView> {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let x0 = CONTENT_PAD_X;
    let lx = x0 + 16.0; // 标签 x
    let cx = x0 + 200.0; // 控件 x
    let cw = COL_W - 200.0 - 16.0; // 控件区宽
    let lw = cx - lx; // 标签列宽(容纳最长标签,不裁剪)
    let mut y = H - CONTENT_PAD_X - TOP_INSET;

    // header:齿轮图标 + 标题(DEV.md General Settings Card 的 icon + Name)
    add_header_icon(
        &pane,
        NSRect::new(NSPoint::new(x0, y + 3.0), NSSize::new(20.0, 20.0)),
        "gearshape",
    );
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 28.0, y),
            NSSize::new(COL_W - 28.0, CONTENT_HEADER_H),
        ),
        st.general,
        false,
        true,
    );
    y -= HEADER_GAP;

    // —— Group-1:语言 + 重置所有(DEV.md「Group 不带名称,仅分组」,顺序即从上至下)——
    add_card(&pane, card_frame(x0, y, 2));
    // Language(标签 + English / 中文 单选;默认中文)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 0) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.language,
        false,
        false,
    );
    add_radio_button(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "English",
        LANG_EN_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    add_radio_button(
        &pane,
        NSRect::new(
            NSPoint::new(cx + 100.0, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "中文",
        LANG_ZH_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    let lang = delegate.ivars().settings.borrow().lang;
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
    // Reset All(按钮 → 确认对话框 → 重置全部自定义:语言 + 各状态灯效)
    let _ = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 1) - 14.0),
            NSSize::new(130.0, 28.0),
        ),
        st.reset_all,
        0,
        sel!(resetAll:),
        delegate,
    );
    y -= card_height(2) + CARD_GAP;

    // —— Group-2:浮窗灯大小 / 点击穿透 / Agent状态轮询间隔 / 开机自启动 ——
    add_card(&pane, card_frame(x0, y, 4));
    // Light size(标签 + 滑块 + 右侧 `xx px` 实时标签)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 0) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.light_size,
        false,
        false,
    );
    let dot = delegate.ivars().settings.borrow().dot_size;
    add_slider(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 0) - 11.0),
            NSSize::new(cw - 60.0, 22.0),
        ),
        8.0,
        40.0,
        dot as f64,
        sel!(changeSize:),
        delegate,
    );
    let size_label = add_text(
        &pane,
        NSRect::new(
            NSPoint::new(cx + cw - 52.0, row_center_y(y, 0) - 10.0),
            NSSize::new(52.0, 20.0),
        ),
        &format!("{} px", dot),
        false,
        false,
    );
    set_tag(&size_label, SIZE_LABEL_TAG);
    // Click-through(标签 + 开关;与 Drop-down「锁定」同步同一开关)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 1) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.click_through,
        false,
        false,
    );
    add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 1) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        *delegate.ivars().click_through.borrow(),
        sel!(toggleClickThrough:),
        delegate,
    );
    // Agent poll interval(标签 + 下拉;1/2/3/5/10/15 秒)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 2) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.poll_interval,
        false,
        false,
    );
    let poll_ms = delegate.ivars().settings.borrow().poll_interval_ms;
    add_popup(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 2) - 13.0),
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
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 3) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.launch_login,
        false,
        false,
    );
    let launch = add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 3) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        false,
        sel!(noop:),
        delegate,
    );
    unsafe {
        let _: () = msg_send![&launch, setEnabled: Bool::NO];
    }

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
    let x0 = CONTENT_PAD_X;
    let lx = x0 + 16.0;
    let cx = x0 + 200.0;
    let cw = COL_W - 200.0 - 16.0;
    let lw = cx - lx;
    let base = tab_of_key(key) * 1000;
    let mut y = H - CONTENT_PAD_X - TOP_INSET;

    // 标题属于右侧内容 panel 的 header;Reset 对齐到同一 header 的尾部。
    add_text(
        &pane,
        NSRect::new(NSPoint::new(x0, y), NSSize::new(COL_W, CONTENT_HEADER_H)),
        name,
        false,
        true,
    );
    let _ = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(CONTENT_W - CONTENT_PAD_X - 70.0, y + 1.0),
            NSSize::new(70.0, 24.0),
        ),
        st.reset,
        base + RESET_OFF,
        sel!(resetStateStyle:),
        delegate,
    );
    y -= HEADER_GAP;

    // —— Card:状态(3 行)——
    add_card(&pane, card_frame(x0, y, 3));
    // Color(标签 + 横向色块;都对齐 row 0 中心)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 0) - 10.0),
            NSSize::new(lw, 20.0),
        ),
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
                NSPoint::new(sx, row_center_y(y, 0) - 12.0),
                NSSize::new(SWATCH_D, SWATCH_D),
            ),
            color,
            base + COLOR_OFF + i as i64,
            delegate,
        ));
    }
    // Animation(标签 + 3 单选;都对齐 row 1 中心)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 1) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.animation,
        false,
        false,
    );
    let mut anim_btns: Vec<Retained<NSButton>> = Vec::with_capacity(3);
    for (i, &nm) in st.anim.iter().enumerate() {
        anim_btns.push(add_radio_button(
            &pane,
            NSRect::new(
                NSPoint::new(cx + i as CGFloat * 76.0, row_center_y(y, 1) - 11.0),
                NSSize::new(72.0, 22.0),
            ),
            nm,
            base + ANIM_OFF + i as i64,
            delegate,
            sel!(changeAnim:),
        ));
    }
    // Speed(标签 + 滑块 + Hz;都对齐 row 2 中心)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 2) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.speed,
        false,
        false,
    );
    let speed = add_slider(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 2) - 11.0),
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
            NSPoint::new(cx + cw - 56.0, row_center_y(y, 2) - 10.0),
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
    let x0 = CONTENT_PAD_X;
    let mut y = H - CONTENT_PAD_X - TOP_INSET;
    add_text(
        &pane,
        NSRect::new(NSPoint::new(x0, y), NSSize::new(COL_W, CONTENT_HEADER_H)),
        st.about,
        false,
        true,
    );
    y -= HEADER_GAP;
    add_card(&pane, card_frame(x0, y, 3));
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 0) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
        "Asig",
        true,
        true,
    );
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 1) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
        &format!("{}{}", st.version, env!("CARGO_PKG_VERSION")),
        true,
        false,
    );
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 2) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
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
