//! Drop-down Panel:单击菜单栏 Signal Icon 后弹出的原生 NSPopover。内容 = 标题 + 三按钮
//! (设置 / 锁定 / 退出)+ 会话列表。NSPopover 自带圆角 + 箭头 + vibrancy 材质 + 失焦自动关
//! (behavior=.transient),故不再自绘 borderless 窗 / CardView / 手算定位。

use agent_light_core::{AgentKind, AgentSession, Snapshot};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSButton, NSFont, NSPopover, NSPopoverBehavior, NSStatusBarButton, NSTextField,
    NSView, NSViewController,
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
    let content = NSView::new(mtm);
    content.setFrame(frame);

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
        // NSButtonType/NSControlStateValue 常量在 NSButtonCell feature 后(未开),用裸值:
        // 3 = NSSwitchButton(圆角勾);1/0 = NSOnState/NSOffState。
        let _: () = msg_send![&btn_lock, setButtonType: 3u64];
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
    let label = NSTextField::labelWithString(&NSString::from_str("(无会话)"), mtm);
    let font = NSFont::systemFontOfSize(12.0);
    label.setFont(Some(&font));
    label.setFrame(NSRect::new(
        NSPoint::new(16.0, 16.0),
        NSSize::new(PANEL_W - 32.0, PANEL_H - 96.0),
    ));
    content.addSubview(&label);

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
    popover.setContentSize(NSSize::new(PANEL_W, PANEL_H));
    popover.setContentViewController(Some(&vc));

    Popover { popover, label }
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
        // showRelativeToRect 的 preferredEdge(NSRectEdge)用 msg_send 传裸值 NSMinYEdge=1,
        // 避开 NSRectEdge 常量与 button 子类 upcast 的不确定性。
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
                    session_id_label(s)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    p.label.setStringValue(&NSString::from_str(&text));
}

/// 会话列表每行的标识:OpenClaw 显示 agent 名(main/munger/kotomi);
/// Claude/CodeBuddy 显示工作目录名(比 session UUID 易读)。
fn session_id_label(s: &AgentSession) -> String {
    match s.kind {
        AgentKind::OpenClaw => s.label.clone().unwrap_or_else(|| "-".into()),
        _ => s
            .cwd
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("-")
            .to_string(),
    }
}

fn add_label(content: &Retained<NSView>, frame: NSRect, text: &str, bold: bool) {
    let mtm = MainThreadMarker::new().expect("add_label 须在主线程");
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    if bold {
        let font = NSFont::boldSystemFontOfSize(14.0);
        label.setFont(Some(&font));
    }
    label.setFrame(frame);
    content.addSubview(&label);
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
