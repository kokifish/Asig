//! Drop-down Panel:单击菜单栏 Signal Icon 后弹出的原生 NSPopover。内容 = 标题 + 三按钮
//! (设置 / 锁定 / 退出)+ 会话列表(**自适应高度,确保每次全显示**)+ 事件列表(可滚动)。
//! NSPopover 自带圆角 + 箭头 + vibrancy 材质 + 失焦自动关(behavior=.transient),故不再
//! 自绘 borderless 窗 / CardView / 手算定位。

use agent_light_core::{EventKind, Snapshot};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSButtonType, NSControlStateValueOff, NSControlStateValueOn, NSFont,
    NSPopover, NSPopoverBehavior, NSScrollView, NSStatusBarButton, NSTextField, NSTextView, NSView,
    NSViewController,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;
use crate::palette::status_emoji;

pub const PANEL_W: f64 = 420.0;

// —— 布局常量(手工 frame;content 非 flipped,y 向上,从底算)——
const PAD: f64 = 16.0; // 左右 + 底 padding
const TOP_PAD: f64 = 12.0; // 标题上方
const TITLE_H: f64 = 18.0;
const BTN_H: f64 = 30.0;
const BTN_GAP: f64 = 8.0; // 标题↔按钮、按钮↔会话列表
const SESS_LINE_H: f64 = 17.0; // 会话每行高(12pt 字体实测 ~17pt;OCR 验证)
const SESS_PAD: f64 = 6.0; // 会话列表文本上下内 padding
const EVENT_GAP: f64 = 8.0; // 会话↔事件
const EVENT_H: f64 = 130.0; // 事件列表区(可滚动)固定高

pub struct Popover {
    popover: Retained<NSPopover>,
    title_label: Retained<NSTextField>,
    btn_settings: Retained<NSButton>,
    btn_lock: Retained<NSButton>,
    btn_quit: Retained<NSButton>,
    sess_label: Retained<NSTextField>,
    event_scroll: Retained<NSScrollView>,
    event_text: Retained<NSTextView>,
}

/// 构建 popover(不显示):标题 + 三按钮 + 会话列表(自适应)+ 事件列表(可滚动)→ VC →
/// NSPopover(transient)。具体高度在 `update_label` 按 snap 重算(build 给 1 行占位初值)。
pub fn build(delegate: &AppDelegate) -> Popover {
    let mtm = MainThreadMarker::new().expect("panel build 须在主线程");
    let content = NSView::new(mtm);
    content.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(PANEL_W, default_h()),
    ));

    // 标题
    let title_label = add_label(
        &content,
        NSRect::new(
            NSPoint::new(PAD, 0.0),
            NSSize::new(PANEL_W - 2.0 * PAD, TITLE_H),
        ),
        "Asig",
        true,
    );

    // —— 顶部三按钮(左→右):设置 / 锁定 / 退出。76pt 宽,在 PANEL_W-32 可用宽内均匀分布 ——
    let btn_settings = add_button(
        &content,
        NSRect::new(NSPoint::new(16.0, 0.0), NSSize::new(76.0, BTN_H)),
        "设置",
        delegate,
        sel!(openSettings:),
    );
    let locked = *delegate.ivars().click_through.borrow(); // 锁定 = 不可拖动 = click_through
    let btn_lock = add_button(
        &content,
        NSRect::new(NSPoint::new(172.0, 0.0), NSSize::new(76.0, BTN_H)),
        "锁定",
        delegate,
        sel!(toggleClickThrough:),
    );
    btn_lock.setButtonType(NSButtonType::Switch); // 圆角勾形 toggle
    btn_lock.setState(if locked {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    let btn_quit = add_button(
        &content,
        NSRect::new(
            NSPoint::new(PANEL_W - 16.0 - 76.0, 0.0),
            NSSize::new(76.0, BTN_H),
        ),
        "退出",
        delegate,
        sel!(quit:),
    );

    // 会话列表(自适应高度,纯展示,不滚动):每行一个会话;frame 由 layout 重算。
    let sess_label = NSTextField::labelWithString(&NSString::from_str("(无会话)"), mtm);
    sess_label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    content.addSubview(&sess_label);

    // 事件列表(可滚动):NSScrollView + NSTextView,固定高 EVENT_H,内容多自动出滚动条。
    // 透明背景 NSTextView + NSClipView,配合 popover vibrancy(不盖一层白底)。
    let event_text = NSTextView::new(mtm);
    event_text.setEditable(false);
    event_text.setSelectable(false);
    event_text.setDrawsBackground(false);
    event_text.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    let event_scroll = NSScrollView::new(mtm);
    event_scroll.setDrawsBackground(false);
    event_scroll.setHasVerticalScroller(true);
    event_scroll.setAutohidesScrollers(true);
    event_scroll.contentView().setDrawsBackground(false);
    event_scroll.setDocumentView(Some(&event_text));
    content.addSubview(&event_scroll);

    // 包 VC → NSPopover(transient:失焦自动关)
    let vc = NSViewController::new(mtm);
    vc.setView(&content);
    let popover = NSPopover::new(mtm);
    // ASIG_NO_HIDE(dev):ApplicationDefined 不随失焦关,便于截图;默认 Transient(失焦自动关)。
    let behavior = if std::env::var("ASIG_NO_HIDE").is_ok() {
        NSPopoverBehavior::ApplicationDefined
    } else {
        NSPopoverBehavior::Transient
    };
    popover.setBehavior(behavior);
    popover.setContentSize(NSSize::new(PANEL_W, default_h()));
    popover.setContentViewController(Some(&vc));

    let p = Popover {
        popover,
        title_label,
        btn_settings,
        btn_lock,
        btn_quit,
        sess_label,
        event_scroll,
        event_text,
    };
    layout(&p, default_sess_h()); // 初始 1 行占位布局
    p
}

/// 锚在状态栏按钮下方弹出 popover。
pub fn show(p: &Popover, button: &NSStatusBarButton) {
    let mtm = MainThreadMarker::new().expect("panel show 须在主线程");
    let rect = button.bounds();
    let app = NSApplication::sharedApplication(mtm);
    // activateIgnoringOtherApps 兼容 macOS 11+(minos);新 activate() 需 14+,故用旧 API。
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    unsafe {
        // showRelativeToRect 的 preferredEdge 取 NSRectEdge;此版本 bindings 未导出
        // NSRectEdge 类型/NSMinYEdge 常量,保留 msg_send! 传裸值 1(=NSMinYEdge,下方)。
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
    unsafe { p.popover.performClose(None) };
}

/// 用最新快照刷新会话列表 + 事件列表,并按会话行数重算 Panel 高度(自适应,确保全显示)。
pub fn update_label(p: &Popover, snap: &Snapshot) {
    // 会话列表:活跃会话的当前状态。
    let sess_text = if snap.sessions.is_empty() {
        "(无活跃会话)".to_string()
    } else {
        snap.sessions
            .iter()
            .map(|s| {
                format!(
                    "{} {} · {}",
                    status_emoji(s.status),
                    s.kind.display_name(),
                    s.display_label()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    p.sess_label.setStringValue(&NSString::from_str(&sess_text));

    // 事件列表:最近 start/done(最新在前)。content 已在内核截断折叠。
    let ev_text = if snap.recent_events.is_empty() {
        "(暂无事件)".to_string()
    } else {
        snap.recent_events
            .iter()
            .map(|e| {
                format!(
                    "{} · {} · {}: {}",
                    e.kind.display_name(),
                    e.label,
                    event_kind_label(e.event_kind),
                    e.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    p.event_text.setString(&NSString::from_str(&ev_text));

    // 会话行数(空占位 1 行)→ 会话列表高度 → 重布局 + 调 popover 大小。
    let lines = snap.sessions.len().max(1) as f64;
    layout(p, lines * SESS_LINE_H + 2.0 * SESS_PAD);
}

/// 按「会话列表高度」重算总高并重定位所有子 view + popover.contentSize。
/// 从底向上排(非 flipped y):event(底)→ gap → sess → gap → buttons → gap → title → top_pad。
fn layout(p: &Popover, sess_h: f64) {
    let event_y = PAD;
    let sess_y = event_y + EVENT_H + EVENT_GAP;
    let btn_y = sess_y + sess_h + BTN_GAP;
    let title_y = btn_y + BTN_H + BTN_GAP;
    let h = title_y + TITLE_H + TOP_PAD;

    p.title_label.setFrame(NSRect::new(
        NSPoint::new(PAD, title_y),
        NSSize::new(PANEL_W - 2.0 * PAD, TITLE_H),
    ));
    p.btn_settings.setFrame(NSRect::new(
        NSPoint::new(16.0, btn_y),
        NSSize::new(76.0, BTN_H),
    ));
    p.btn_lock.setFrame(NSRect::new(
        NSPoint::new(172.0, btn_y),
        NSSize::new(76.0, BTN_H),
    ));
    p.btn_quit.setFrame(NSRect::new(
        NSPoint::new(PANEL_W - 16.0 - 76.0, btn_y),
        NSSize::new(76.0, BTN_H),
    ));
    p.sess_label.setFrame(NSRect::new(
        NSPoint::new(PAD, sess_y),
        NSSize::new(PANEL_W - 2.0 * PAD, sess_h),
    ));
    p.event_scroll.setFrame(NSRect::new(
        NSPoint::new(PAD, event_y),
        NSSize::new(PANEL_W - 2.0 * PAD, EVENT_H),
    ));
    p.popover.setContentSize(NSSize::new(PANEL_W, h));
}

/// 默认(无 snap)总高:1 行会话占位。
fn default_h() -> f64 {
    let sess_y = PAD + EVENT_H + EVENT_GAP;
    sess_y + default_sess_h() + BTN_GAP + BTN_H + BTN_GAP + TITLE_H + TOP_PAD
}

/// 默认会话列表高度(1 行占位)。
fn default_sess_h() -> f64 {
    SESS_LINE_H + 2.0 * SESS_PAD
}

/// EventKind → 展示名(Panel 事件列表行用)。
fn event_kind_label(k: EventKind) -> &'static str {
    match k {
        EventKind::Start => "start",
        EventKind::Done => "done",
    }
}

fn add_label(
    content: &Retained<NSView>,
    frame: NSRect,
    text: &str,
    bold: bool,
) -> Retained<NSTextField> {
    let mtm = MainThreadMarker::new().expect("add_label 须在主线程");
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    if bold {
        label.setFont(Some(&NSFont::boldSystemFontOfSize(14.0)));
    }
    label.setFrame(frame);
    content.addSubview(&label);
    label
}

/// 建一个普通按钮:frame / title / target / action 一次配齐并加到 content;返回它供进一步定制。
fn add_button(
    content: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    delegate: &AppDelegate,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_button 须在主线程");
    let btn = NSButton::new(mtm);
    btn.setFrame(frame);
    btn.setTitle(&NSString::from_str(title));
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(action));
    }
    content.addSubview(&btn);
    btn
}
