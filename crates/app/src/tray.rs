//! 菜单栏灯:NSStatusItem + 自绘彩色圆点按钮。点击按钮弹 popover(见 panel.rs)。
//! 菜单栏无「深绿圆」emoji,故按钮图标用自绘 NSImage 圆点 —— 所有状态统一为「仅颜色不同」
//! 的圆(Done 绿 / DoneNotif 深绿 / Working 黄 …)。

use agent_light_core::{AgentStatus, Color, LightAnim};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::Bool;
use objc2::{DefinedClass, MainThreadMarker, class, msg_send, sel};
use objc2_app_kit::{NSBezierPath, NSImage, NSStatusBar, NSStatusItem};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSTimer};

use crate::app_delegate::AppDelegate;
use crate::overlay::nscolor;

/// 建状态栏项,并把按钮点击接到 `togglePopover:`。须在主线程调用(button() 要 MainThreadMarker)。
pub fn build(delegate: &Retained<AppDelegate>) {
    // MainThreadMarker:证明当前在主线程。NSApplication 启动期在主线程,故 new() 安全。
    let mtm = MainThreadMarker::new().expect("build 须在主线程");
    let sb = NSStatusBar::systemStatusBar();
    let item = sb.statusItemWithLength(-1.0); // -1 = NSVariableStatusItemLength(自适应宽度)
    set_light(&item, &AgentStatus::Offline.light(), mtm);

    // 点状态栏按钮 → 弹/收 popover
    let button = item.button(mtm).expect("状态栏按钮");
    unsafe {
        button.setTarget(Some(&**delegate));
        button.setAction(Some(sel!(togglePopover:)));
    }

    delegate.ivars().status_item.replace(Some(item));
}

/// 按灯效(颜色)把按钮图标设成自绘圆点。
pub fn set_light(item: &NSStatusItem, anim: &LightAnim, mtm: MainThreadMarker) {
    let color = match anim {
        LightAnim::Steady { color } => *color,
        LightAnim::Pulse { color, .. } => *color,
        LightAnim::Ripple { color, .. } => *color,
    };
    let button = item.button(mtm).expect("状态栏按钮");
    let img = circle_image(color);
    unsafe {
        let _: () = msg_send![&button, setImage: &*img];
    }
}

/// 画一个 `color` 的实心圆 NSImage(菜单栏用)。setTemplate:NO 保留真彩(否则菜单栏按
/// 模板渲染成单色灰)。
fn circle_image(c: Color) -> Retained<NSImage> {
    const S: CGFloat = 18.0;
    let alloc: Allocated<NSImage> = unsafe { msg_send![class!(NSImage), alloc] };
    let img: Retained<NSImage> = unsafe { msg_send![alloc, initWithSize: NSSize::new(S, S)] };
    unsafe {
        let _: () = msg_send![&img, setTemplate: Bool::NO];
        let _: () = msg_send![&img, lockFocus];
        let inset: CGFloat = 2.0;
        let rect = NSRect::new(
            NSPoint::new(inset, inset),
            NSSize::new(S - inset * 2.0, S - inset * 2.0),
        );
        let path: Retained<NSBezierPath> =
            msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: rect];
        let col = nscolor(c);
        let _: () = msg_send![&col, set];
        path.fill();
        let _: () = msg_send![&img, unlockFocus];
    }
    img
}

/// 启动 tick 定时器:间隔取自设置(默认 3s)。timer 存 ivars,以便运行时按新间隔重排。
pub fn schedule_tick(delegate: &Retained<AppDelegate>) {
    let interval = delegate.ivars().settings.borrow().poll_interval_ms as f64 / 1000.0;
    reschedule(delegate, interval);
}

/// 重排 tick 定时器:作废旧 timer、按新间隔建新的(轮询间隔改动后调用)。
pub fn reschedule(delegate: &AppDelegate, interval: f64) {
    if let Some(old) = delegate.ivars().tick_timer.borrow_mut().take() {
        let _: () = unsafe { msg_send![&old, invalidate] };
    }
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            interval,
            delegate,
            sel!(tick:),
            None,
            true,
        )
    };
    *delegate.ivars().tick_timer.borrow_mut() = Some(timer);
}
