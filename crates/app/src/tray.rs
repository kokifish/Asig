//! 菜单栏灯:NSStatusItem + 按钮 emoji。点击按钮弹 popover(见 panel.rs)。
//! 用 objc2 0.6 的强类型方法(NSStatusBar::systemStatusBar / item.button(mtm) /
//! button.setTarget / NSTimer::scheduledTimer...)替代手写 msg_send!。

use agent_light_core::{AgentStatus, LightAnim};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, sel};
use objc2_app_kit::{NSStatusBar, NSStatusItem};
use objc2_foundation::{NSString, NSTimer};

use crate::app_delegate::AppDelegate;
use crate::palette::color_emoji;

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

/// 按灯效(颜色)把按钮标题设成对应 emoji。Done Notification 是深绿 → 💚。
pub fn set_light(item: &NSStatusItem, anim: &LightAnim, mtm: MainThreadMarker) {
    let color = match anim {
        LightAnim::Steady { color } => *color,
        LightAnim::Pulse { color, .. } => *color,
        LightAnim::Ripple { color, .. } => *color,
    };
    let button = item.button(mtm).expect("状态栏按钮");
    button.setTitle(&NSString::from_str(color_emoji(color)));
}

/// NSTimer scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:
pub fn schedule_tick(delegate: &Retained<AppDelegate>) {
    let interval = agent_light_core::Monitor::poll_interval().as_secs_f64();
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            interval,
            delegate,
            sel!(tick:),
            None,
            true,
        )
    };
    std::mem::forget(timer);
}
