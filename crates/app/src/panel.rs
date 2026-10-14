//! Drop-down Panel:单击菜单栏 Signal Icon 后弹出的原生 NSPopover。内容 = 标题 + 三按钮
//! (设置 / 锁定 / 退出)+ 会话列表 + 事件列表(可滚动)。会话列表硬上限 20 行(超出滚动);
//! 事件列表无行数上限,高度随内容自适应(内容少时收缩不留空白),只受所在屏可视高度的
//! 系统硬上限约束 —— 超过才滚动。
//! NSPopover 自带圆角 + 箭头 + vibrancy 材质 + 失焦自动关(behavior=.transient),故不再
//! 自绘 borderless 窗 / CardView / 手算定位。

use agent_light_core::{EventKind, Snapshot};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSButtonType, NSControlStateValueOff, NSControlStateValueOn, NSFont,
    NSPopover, NSPopoverBehavior, NSScreen, NSScrollView, NSScrollerStyle, NSStatusBarButton,
    NSTextField, NSTextView, NSView, NSViewController,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;

use crate::app_delegate::AppDelegate;
use crate::palette::session_emoji;

pub const PANEL_W: f64 = 420.0;
/// 内容区宽(布局与滚动文档统一用,防各自手算漂移)。
const CONTENT_W: f64 = PANEL_W - 2.0 * PAD;

// —— 布局常量(手工 frame;content 非 flipped,y 向上,从底算)——
const PAD: f64 = 16.0; // 左右 + 底 padding
const TOP_PAD: f64 = 12.0; // 标题上方
const TITLE_H: f64 = 18.0;
const BTN_H: f64 = 30.0;
const BTN_GAP: f64 = 8.0; // 标题↔按钮、按钮↔会话列表
const SESS_LINE_H: f64 = 17.0; // 会话每行高(12pt 字体实测 ~17pt;OCR 验证)
const SESS_PAD: f64 = 6.0; // 会话列表文本上下内 padding
const EVENT_GAP: f64 = 8.0; // 会话↔事件
/// 事件列表最小高(收缩下限):屏幕预算被会话列表吃满时,事件区至少留 2 行可看。
const EVENT_MIN_LINES: f64 = 2.0;
/// 会话列表平铺行数硬上限:最多 20 行,超出部分进滚动区(只有滚动可见)。
const SESS_MAX_ROWS: f64 = 20.0;
/// 面板总高距所在屏可视区的安全余量(菜单栏等)。
const SCREEN_MARGIN: f64 = 24.0;

pub struct Popover {
    popover: Retained<NSPopover>,
    title_label: Retained<NSTextField>,
    btn_settings: Retained<NSButton>,
    btn_lock: Retained<NSButton>,
    btn_quit: Retained<NSButton>,
    sess_scroll: Retained<NSScrollView>,
    sess_text: Retained<NSTextView>,
    /// 上次会话列表文本:仅在变化时回滚到顶(轮询 tick 不打扰用户的滚动位置)。
    sess_text_cache: RefCell<String>,
    event_scroll: Retained<NSScrollView>,
    event_text: Retained<NSTextView>,
    /// 同上,事件列表的回顶去重缓存。
    event_text_cache: RefCell<String>,
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

    // 会话列表(自适应高度,硬上限 20 行,超出滚动):每行一个会话;frame 由 layout 重算。
    // 透明背景不盖 vibrancy;autohides scrollers —— 内容放得下时无滚动条,观感同平铺。
    let sess_text = NSTextView::new(mtm);
    sess_text.setEditable(false);
    sess_text.setSelectable(false);
    sess_text.setDrawsBackground(false);
    sess_text.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    // 关掉自调高:frame 全由 update_label 按行数显式给值 —— 否则 setString 时它先按内部
    // 预估自调,最终高度要等绘制才落定,滚动回顶会按过期高度算错。
    sess_text.setVerticallyResizable(false);
    // 两滚动视图显式钉死 overlay 滚动条:legacy 风格(系统设置「总是显示滚动条」时会
    // 被选中)在布局里预留 ~17pt 宽,文档宽度在 388↔371 间跳变 → 文本重折行,按全宽
    // 量出的内容高失真 → 内容少也滚出滚动条。overlay 不占布局宽,滚动时浮现、松手淡出
    // (现代 macOS 的系统兜底行为),高度测量也随之稳定。
    let sess_scroll = NSScrollView::new(mtm);
    sess_scroll.setDrawsBackground(false);
    sess_scroll.setHasVerticalScroller(true);
    sess_scroll.setAutohidesScrollers(true);
    sess_scroll.setScrollerStyle(NSScrollerStyle::Overlay);
    sess_scroll.contentView().setDrawsBackground(false);
    sess_scroll.setDocumentView(Some(&sess_text));
    content.addSubview(&sess_scroll);

    // 事件列表(可滚动):NSScrollView + NSTextView,高度随内容自适应(无行数上限,
    // 只受屏幕预算约束),内容多自动出滚动条。透明背景 NSTextView + NSClipView,配合
    // popover vibrancy。
    let event_text = NSTextView::new(mtm);
    event_text.setEditable(false);
    event_text.setSelectable(false);
    event_text.setDrawsBackground(false);
    event_text.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    // 纵向随内容自调(sizeToFit 量高用);横向锁宽 —— 否则 setString 会把宽度自调到
    // 含滚动条余量的 388,与布局宽漂移。
    event_text.setVerticallyResizable(true);
    event_text.setHorizontallyResizable(false);
    event_text.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, SESS_LINE_H),
    ));
    let event_scroll = NSScrollView::new(mtm);
    event_scroll.setDrawsBackground(false);
    event_scroll.setHasVerticalScroller(true);
    event_scroll.setAutohidesScrollers(true);
    event_scroll.setScrollerStyle(NSScrollerStyle::Overlay);
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
        sess_scroll,
        sess_text,
        sess_text_cache: RefCell::new(String::new()),
        event_scroll,
        event_text,
        event_text_cache: RefCell::new(String::new()),
    };
    layout(&p, default_sess_h(), default_ev_h()); // 初始 1 行占位布局
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

/// 用最新快照刷新会话列表 + 事件列表,并按内容重算 Panel 高度。会话列表硬上限 20 行
/// (超出滚动);事件列表无行数上限,随内容自适应,只受所在屏可视高度的系统硬上限约束。
pub fn update_label(p: &Popover, snap: &Snapshot) {
    // 会话列表:活跃会话的当前状态。
    let text = if snap.sessions.is_empty() {
        "(无活跃会话)".to_string()
    } else {
        snap.sessions
            .iter()
            .map(|s| {
                format!(
                    "{} {} · {}",
                    // 只有「自己刚转入 Done」(窗口内)的行显 🔵;别的会话触发的全局完成通知
                    // 不再把闲置已久的 Done 行刷蓝(见 Snapshot::done_notif_ids)。
                    session_emoji(s.status, snap.done_notif_ids.contains(&s.id)),
                    s.kind.display_name(),
                    s.display_label()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    // NSTextView 文档非翻转,内容超高时初始停在文档底 —— 文本变化(含首刷)把滚动位定回
    // 顶部;没变的轮询 tick 不动,保留用户滚动位置。不用 scrollRangeToVisible:TextKit
    // 延迟布局下是空操作,滚动位在 layout 后按几何显式定(见函数尾)。
    let sess_changed = {
        let mut prev = p.sess_text_cache.borrow_mut();
        let changed = *prev != text;
        if changed {
            *prev = text.clone();
        }
        changed
    };

    // 事件列表:最近 start/done(最新在前)。content 已在内核截断折叠。
    // 高度按 TextKit 实测内容自适应(折行也算准),变化时回顶。
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
    let ev_changed = {
        let mut prev = p.event_text_cache.borrow_mut();
        let changed = *prev != ev_text;
        if changed {
            *prev = ev_text.clone();
        }
        changed
    };
    let ev_needed = fitted_height(&p.event_text, &ev_text);

    // 会话行数(空占位 1 行)→ 需要的列表高度。滚动文档 frame 显式给值(不赌 TextKit
    // 自动增长/延迟布局),行高同一实测值。
    let lines = snap.sessions.len().max(1) as f64;
    let sess_needed = (lines * SESS_LINE_H + 2.0 * SESS_PAD).max(default_sess_h());
    // 两区高度分配:会话硬上限 20 行;事件无行数上限,只受屏幕预算(系统硬上限)约束。
    let (sess_h, ev_h) = split_areas(sess_needed, ev_needed, list_budget());
    p.sess_text.setString(&NSString::from_str(&text));
    p.sess_text.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, sess_needed),
    ));
    // layout 重设各视图 frame / popover 尺寸时,clip 会按底边锚定拖动滚动位(实测随
    // 视口高度变化漂移)—— 记住布局前的滚动位,布局后恢复;文本变化的 tick 例外(回顶)。
    let sess_origin = p.sess_scroll.contentView().bounds().origin;
    let ev_origin = p.event_scroll.contentView().bounds().origin;
    layout(p, sess_h, ev_h);
    // 回顶 = clip bounds 原点 (0,0)(实测 NSTextView 作文档时原点在顶部 —— flipped
    // 语义;setBoundsOrigin 不受 TextKit 延迟布局影响,几何即时生效)。文本变化(含首刷)
    // 回顶;没变的 tick 恢复布局前滚动位(抵消 layout 的锚定拖动),用户滚动不受影响。
    if sess_changed {
        p.sess_scroll
            .contentView()
            .setBoundsOrigin(NSPoint::new(0.0, 0.0));
    } else {
        let y = sess_origin.y.clamp(0.0, (sess_needed - sess_h).max(0.0));
        p.sess_scroll
            .contentView()
            .setBoundsOrigin(NSPoint::new(sess_origin.x, y));
    }
    if ev_changed {
        p.event_scroll
            .contentView()
            .setBoundsOrigin(NSPoint::new(0.0, 0.0));
    } else {
        let y = ev_origin.y.clamp(0.0, (ev_needed - ev_h).max(0.0));
        p.event_scroll
            .contentView()
            .setBoundsOrigin(NSPoint::new(ev_origin.x, y));
    }
}

/// setString + sizeToFit 量出文本实际内容高(同步触发 TextKit 布局,折行/容器 inset 都
/// 含在内),再把文档 frame 归一(原点 (0,0)、宽 CONTENT_W)后返回高度。调方把返回值
/// `min(上限)` 后作滚动视口高 —— 内容不足时视口随之收缩,不留空白。
fn fitted_height(tv: &Retained<NSTextView>, text: &str) -> f64 {
    tv.setString(&NSString::from_str(text));
    tv.sizeToFit();
    let h = tv.frame().size.height.max(SESS_LINE_H);
    tv.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, h),
    ));
    h
}

/// 按「会话列表高 + 事件列表高」重算总高并重定位所有子 view + popover.contentSize。
/// 从底向上排(非 flipped y):event(底)→ gap → sess → gap → buttons → gap → title → top_pad。
fn layout(p: &Popover, sess_h: f64, ev_h: f64) {
    let event_y = PAD;
    let sess_y = event_y + ev_h + EVENT_GAP;
    let btn_y = sess_y + sess_h + BTN_GAP;
    let title_y = btn_y + BTN_H + BTN_GAP;
    let h = title_y + TITLE_H + TOP_PAD;

    p.title_label.setFrame(NSRect::new(
        NSPoint::new(PAD, title_y),
        NSSize::new(CONTENT_W, TITLE_H),
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
    p.sess_scroll.setFrame(NSRect::new(
        NSPoint::new(PAD, sess_y),
        NSSize::new(CONTENT_W, sess_h),
    ));
    p.event_scroll.setFrame(NSRect::new(
        NSPoint::new(PAD, event_y),
        NSSize::new(CONTENT_W, ev_h),
    ));
    p.popover.setContentSize(NSSize::new(PANEL_W, h));
}

/// 默认(无 snap)总高:1 行会话 + 1 行事件占位。
fn default_h() -> f64 {
    let sess_y = PAD + default_ev_h() + EVENT_GAP;
    sess_y + default_sess_h() + BTN_GAP + BTN_H + BTN_GAP + TITLE_H + TOP_PAD
}

/// 默认会话列表高度(1 行占位)。
fn default_sess_h() -> f64 {
    SESS_LINE_H + 2.0 * SESS_PAD
}

/// 默认事件列表高度(1 行占位;首次 update 即按内容重算)。
fn default_ev_h() -> f64 {
    SESS_LINE_H
}

/// 会话列表平铺高度硬上限:20 行 × 行高 + 上下 padding。
fn sess_h_cap() -> f64 {
    SESS_MAX_ROWS * SESS_LINE_H + 2.0 * SESS_PAD
}

/// 两区总高预算:所在屏 `visibleFrame` 高 − 安全余量 − 固定 chrome。取不到屏幕(理论
/// 不发生)→ None 不设限,保持全平铺。
fn list_budget() -> Option<f64> {
    let mtm = MainThreadMarker::new().expect("panel 布局须在主线程");
    let screen_h = NSScreen::mainScreen(mtm).map(|s| s.visibleFrame().size.height);
    Some(screen_h? - SCREEN_MARGIN - fixed_chrome_h())
}

/// 除两区列表外的固定 chrome 总高(与 `layout` 的自底向上算式一一对应)。
fn fixed_chrome_h() -> f64 {
    PAD + EVENT_GAP + BTN_GAP * 2.0 + BTN_H + TITLE_H + TOP_PAD
}

/// 两区高度分配(纯函数,可测):**会话列表硬上限 20 行**(`sess_h_cap`),超出部分进
/// 滚动区;**事件列表无行数上限**,高度随内容自适应,只受屏幕预算(= macOS 允许的最大
/// 高度)约束 —— 不超预算全展示,超过则收到剩余预算、区内滚动(极小时保 2 行下限)。
/// `budget=None` = 无屏幕,事件区也不设限。极小预算退化:会话至少 1 行、事件至少下限,
/// 布局不塌(总高可能超屏,属极端环境可接受)。
fn split_areas(sess_needed: f64, ev_needed: f64, budget: Option<f64>) -> (f64, f64) {
    let ev_floor = EVENT_MIN_LINES * SESS_LINE_H;
    let sess_h = match budget {
        // 会话:内容高,封顶 20 行;预算极小时让到至少 1 行占位(布局不塌)。
        Some(b) => sess_needed
            .min(sess_h_cap())
            .min((b - ev_floor).max(default_sess_h())),
        None => sess_needed.min(sess_h_cap()),
    };
    let ev_h = match budget {
        // 事件:无行数上限。预算内按内容原样(1 行就 1 行高,不强撑到下限);
        // 超过剩余预算(macOS 允许的最大高度)才收缩,收缩不低于 2 行下限。
        Some(b) => {
            let remaining = (b - sess_h).max(0.0);
            if ev_needed <= remaining {
                ev_needed
            } else {
                remaining.max(ev_floor)
            }
        }
        None => ev_needed,
    };
    (sess_h, ev_h)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_areas_fits_within_budget() {
        // 预算充足:需求 ≤ 上限时各自全展示,不收缩、不强撑 —— 内容 1 行就 1 行高。
        let (sess, ev) = split_areas(300.0, 200.0, Some(2000.0));
        assert!((sess - 300.0).abs() < 1e-9);
        assert!((ev - 200.0).abs() < 1e-9);
        let (sess, ev) = split_areas(300.0, EVENT_MIN_LINES * SESS_LINE_H - 5.0, Some(2000.0));
        assert!((sess - 300.0).abs() < 1e-9);
        assert!(
            (ev - (EVENT_MIN_LINES * SESS_LINE_H - 5.0)).abs() < 1e-9,
            "预算内不应把事件区强撑到下限"
        );
    }

    #[test]
    fn split_areas_session_hard_cap_twenty_rows() {
        // 会话列表硬上限 20 行:无论预算多充裕,超出 20 行的部分进滚动区。
        let (sess, ev) = split_areas(80.0 * SESS_LINE_H + 2.0 * SESS_PAD, 100.0, Some(2000.0));
        assert!((sess - sess_h_cap()).abs() < 1e-9, "会话应封顶 20 行");
        assert!(sess < 80.0 * SESS_LINE_H);
        assert!((ev - 100.0).abs() < 1e-9);
        // 无屏幕(None)时会话硬上限依然生效。
        let (sess, ev) = split_areas(80.0 * SESS_LINE_H + 2.0 * SESS_PAD, 100.0, None);
        assert!((sess - sess_h_cap()).abs() < 1e-9);
        assert!((ev - 100.0).abs() < 1e-9);
    }

    #[test]
    fn split_areas_event_unbounded_until_screen() {
        // 事件列表无行数上限:内容再高,预算内也全展示(视口=内容高,不出滚动条);
        // 只有超过 macOS 允许的最大高度(剩余预算)才压缩、出滚动条。
        let big_ev = 60.0 * SESS_LINE_H; // 60 行事件,远超旧行数上限语义
        let (_sess, ev) = split_areas(default_sess_h(), big_ev, Some(2000.0));
        assert!((ev - big_ev).abs() < 1e-9, "预算内事件应全展示");
        // 超预算:事件收到剩余预算,区内滚动;总高不超预算。
        let (sess, ev) = split_areas(default_sess_h(), big_ev, Some(1000.0));
        assert!(ev < big_ev, "超预算事件应收缩");
        assert!(sess + ev <= 1000.0 + 1e-9);
        assert!(ev >= EVENT_MIN_LINES * SESS_LINE_H, "收缩不低于 2 行下限");
        // 无屏幕(None):事件完全不设限。
        let (_sess, ev) = split_areas(default_sess_h(), big_ev, None);
        assert!((ev - big_ev).abs() < 1e-9);
    }

    #[test]
    fn split_areas_tiny_budget_degrades_gracefully() {
        // 极小预算(理论极端环境):会话至少 1 行占位、事件至少下限,布局不出现负高。
        let (sess, ev) = split_areas(500.0, 300.0, Some(10.0));
        assert!((sess - default_sess_h()).abs() < 1e-9);
        assert!((ev - EVENT_MIN_LINES * SESS_LINE_H).abs() < 1e-9);
    }
}
