//! 设置窗口(左侧栏导航)。界面文案按 `Settings.lang`(默认中文)本地化,可切全英文。
//! 左栏:General + 6 状态 tab(左对齐;状态 tab = 当前色圆点 + 单语言名称——按 `lang` 取 DEV.md
//! 「Signal Color」表「状态名称」列的中文或英文其中一档,不双语并排)+ 底部单色 SF Symbol
//! 图标行(关于 functional;其余占位禁用)。右区:8 pane。
//! 状态 pane = State Settings Card(Reset + Color 色块单选 + Animation 单选 + Speed Hz),
//! 颜色/动画/速度各占一行。
//!
//! 本模块拆成 9 个子模块(纯结构重构,行为零变化):
//! - `strings`:界面文案(`Strings` / `strings_for` / `reset_confirm_texts`)。
//! - `tags`:几何常量、tag 编码与几何 helper。
//! - `controls`:控件工厂(add_*)。
//! - `glass`:液态玻璃 / vibrancy 后端 + 侧栏选中态药丸 + tab 选中切换。
//! - `layout`:`StateControls` + 按窗宽重排 / 按样式刷新。
//! - `pane_general` / `pane_state` / `pane_about`:各 pane 构造。
//! - 本文件(`mod`):装配(build/show/sidebar)+ `pub use` 重导出外部所需 API。

use std::collections::HashMap;

use objc2::DefinedClass;
use objc2::rc::{Allocated, Retained};
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSColor, NSView, NSWindow};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

use controls::{add_icon_button, add_tab_button, new_view};
use glass::{glass_pane, make_selection_pill};
use pane_about::build_about_pane;
use pane_general::build_general_pane;
use pane_state::build_state_pane;
use strings::{Strings, strings_for};
use tags::{STATE_KEYS, sf_symbol};

// 子模块声明。
mod controls;
mod glass;
mod layout;
mod pane_about;
mod pane_general;
mod pane_state;
mod strings;
mod tags;

// ---- 外部(crate::settings::)所需的 pub use 重导出 ----
pub use glass::update_selection;
pub use layout::{
    StateControls, layout_state_pane, refresh_agent_chips, refresh_duration, refresh_state_controls,
};
pub use strings::reset_confirm_texts;
// pub const / pub fn 重导出(保持原 pub 可见性 —— 即使当前外部未全部引用,
// 拆分前它们都是 settings 的公开 API)。`#[allow(unused_imports)]` 抑制
// 「重导出的符号本 crate 内未通过此路径使用」的告警(clippy -D warnings)。
#[allow(unused_imports)]
pub use tags::{
    AGENT_KIND_ORDER, AGENT_OFF, ANIM_OFF, ANIM_ORDER, COLOR_OFF, COLOR_ORDER, CONTENT_W,
    LANG_EN_TAG, LANG_ZH_TAG, POLL_PRESETS_MS, RESET_OFF, SIZE_LABEL_TAG, SPEED_LABEL_OFF,
    SPEED_MAX, SPEED_MIN, SPEED_OFF, TAB_ABOUT, TAB_GENERAL, THEME_OFF, parse_control_tag,
    stylekey_of_tab,
};

pub fn build(delegate: &AppDelegate) -> Retained<NSWindow> {
    let lang = delegate.ivars().settings.borrow().lang;
    let st = strings_for(lang);

    // 窗口:titled(1)|closable(2)|miniaturizable(4)|resizable(8)|fullSizeContentView(32768)
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(tags::W, tags::H));
    // initWithContentRect / styleMask 常量在 feature 后,保留 msg_send!(透传避开子类 upcast)。
    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: 32783u64,
            backing: 2u64,
            defer: false,
        ]
    };
    window.setTitle(&NSString::from_str("Asig"));
    unsafe {
        // setReleasedWhenClosed: ARC 下手动 retain,需 unsafe。
        let _: () = msg_send![&window, setReleasedWhenClosed: false];
    }
    window.setOpaque(false); // 透明底,让 vibrancy 能模糊桌面
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setTitlebarAppearsTransparent(true); // 内容贯穿标题栏
    unsafe {
        // setTitleVisibility / setTitlebarSeparatorStyle 取 enum 常量(feature 后),保留裸值 msg_send!。
        let _: () = msg_send![&window, setTitleVisibility: 1i64]; // hidden
        let _: () = msg_send![&window, setTitlebarSeparatorStyle: 1i64]; // none — 玻璃贯穿标题栏,无顶部分隔线
    }
    window.setMovable(true);
    window.setMinSize(NSSize::new(tags::W, tags::H));
    // AppDelegate 兼作窗口 delegate:windowDidResize: 触发 state pane 色块按新宽度重排。
    // setDelegate 取 ProtocolObject<dyn NSWindowDelegate>,构造繁琐,保留 msg_send!。
    unsafe {
        let _: () = msg_send![&window, setDelegate: delegate];
    }

    // 右区:透明 NSView,8 pane 叠在其上。origin 在 SIDEBAR_W,铺在主玻璃上(无外框)。
    let content_area = new_view(NSRect::new(
        NSPoint::new(tags::SIDEBAR_W, 0.0),
        NSSize::new(CONTENT_W, tags::H),
    ));
    // 宽+高 随窗口缩放(左侧栏固定宽,故右区宽度 = 窗宽 − SIDEBAR_W)。
    content_area.setAutoresizingMask(NSAutoresizingMaskOptions(18));
    // 8 pane:General + 6 状态(各带 StateControls)+ About。按 pane id(=索引)排。
    let mut panes: Vec<Retained<NSView>> = Vec::with_capacity(8);
    let mut controls_map: HashMap<agent_light_core::StyleKey, StateControls> = HashMap::new();
    panes.push(build_general_pane(delegate, &st));
    for (i, (_, key)) in STATE_KEYS.iter().enumerate() {
        let (pane, c) = build_state_pane(delegate, *key, st.state[i], &st);
        controls_map.insert(*key, c);
        panes.push(pane);
    }
    panes.push(build_about_pane(&st));
    for (i, pane) in panes.iter().enumerate() {
        pane.setHidden(i != 0);
        // 每个 pane 宽+高 随右区缩放;pane 内的卡片/滑块各自按 autoresizing 适配。
        pane.setAutoresizingMask(NSAutoresizingMaskOptions(18));
        content_area.addSubview(pane);
    }

    // 真·液态玻璃承载视图 root(普通 NSView;刻意不用 NSGlassEffectContainerView —— 它会把
    // 重叠/相邻的玻璃合并成一次模糊,令浮动侧栏失去层次)。root 内:主玻璃(满窗,承载右区内容)
    // + 浮动侧栏玻璃(左侧圆角,承载 tab/图标)两块独立玻璃叠放;侧栏因四周留白 + 二次模糊
    // 读作浮动玻璃面板,内容在主玻璃上无外框。
    let full = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(tags::W, tags::H));
    let root = new_view(full);
    let main = glass_pane(full, 0.0, 12); // 主玻璃满窗(窗口自裁圆角);回退 material=WindowBackground
    let sidebar = glass_pane(
        NSRect::new(
            NSPoint::new(tags::SIDEBAR_INSET, tags::SIDEBAR_INSET),
            NSSize::new(tags::SIDEBAR_PANE_W, tags::SIDEBAR_PANE_H),
        ),
        14.0, // 浮动玻璃圆角
        7,    // 回退 material=Sidebar
    );
    // 侧栏 UI 建到浮动玻璃的 contentView 上。
    build_sidebar(&sidebar.content, delegate, &st);

    main.view.setAutoresizingMask(NSAutoresizingMaskOptions(18)); // 主玻璃随窗口缩放
    root.addSubview(&main.view); // 主玻璃在底
    main.content.addSubview(&content_area); // 右区在主玻璃上
    sidebar
        .view
        .setAutoresizingMask(NSAutoresizingMaskOptions(16)); // 侧栏固定宽,随高伸缩
    root.addSubview(&sidebar.view); // 浮动侧栏在上
    window.setContentView(Some(&root));

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
                    if let Some(p0) = v.first() {
                        p0.setHidden(true);
                    }
                    if let Some(pn) = v.get(n as usize) {
                        pn.setHidden(false);
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
    sidebar.addSubview(&pill);
    *delegate.ivars().settings_selection.borrow_mut() = Some(pill);

    let tab_w = tags::SIDEBAR_PANE_W - 16.0;
    let top = tags::SIDEBAR_PANE_H - 14.0 - 28.0; // 顶部留白 14 + tab 高 28
    // General tab = 齿轮(template SF Symbol)+ 常规设置;选中时 update_selection 把齿轮转白。
    let gear = sf_symbol("gearshape");
    gear.setTemplate(true);
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
    let icon_w = (tags::SIDEBAR_PANE_W - 16.0) / icons.len() as CGFloat;
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
            btn.setEnabled(false);
        }
    }
}

/// content view 里按 tag 找子视图(仅侧栏 tab 按钮;状态控件用 StateControls)。
pub fn view_with_tag(view: &Retained<NSView>, tag: i64) -> Option<Retained<NSView>> {
    view.viewWithTag(tag as isize)
}

pub fn show(window: &NSWindow) {
    let mtm = MainThreadMarker::new().expect("settings::show 须主线程");
    let app = NSApplication::sharedApplication(mtm);
    // activateIgnoringOtherApps 兼容 macOS 11+(minos);新 activate() 需 14+,故用旧 API。
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    window.center();
    window.makeKeyAndOrderFront(None);
}
