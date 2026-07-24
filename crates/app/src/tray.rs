//! 菜单栏灯:NSStatusItem + 自绘彩色圆点按钮。点击按钮弹 popover(见 panel.rs)。
//! 菜单栏无「浅蓝圆」emoji,故按钮图标用自绘 NSImage 圆点(overlay::swatch_image)——
//! 所有状态统一为「仅颜色不同」的圆(Done 绿 / DoneNotif 浅蓝 / Working 黄 …)。

use agent_light_core::{AgentStatus, LightAnim};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusBarButton, NSStatusItem};
use objc2_foundation::{NSPoint, NSString, NSTimer};

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

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
    let button = item.button(mtm).expect("状态栏按钮");
    let img = swatch_image(anim.color(), 18.0, false);
    button.setImage(Some(&img));
}

/// 重排 tick 定时器:作废旧 timer、按新间隔建新的。启动(取设置的默认间隔)与轮询间隔改动共用。
pub fn reschedule(delegate: &AppDelegate, interval: f64) {
    if let Some(old) = delegate.ivars().tick_timer.borrow_mut().take() {
        old.invalidate();
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

/// 状态栏右键菜单:设置… / (分隔) / 退出 Asig。锚在状态栏按钮下方弹出。
pub fn show_status_menu(delegate: &AppDelegate, button: &NSStatusBarButton, mtm: MainThreadMarker) {
    let menu: Retained<NSMenu> = NSMenu::new(mtm);
    unsafe {
        let s = menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str("设置…"),
            Some(sel!(openSettings:)),
            &NSString::from_str(""),
        );
        s.setTarget(Some(delegate));
        let sep = NSMenuItem::separatorItem(mtm);
        menu.addItem(&sep);
        let q = menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str("退出 Asig"),
            Some(sel!(quit:)),
            &NSString::from_str(""),
        );
        q.setTarget(Some(delegate));
    }
    // popUpMenu:view 参数取 Option<&NSView>;NSStatusBarButton 是 NSView 子类,可直接传入。
    menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(0.0, 0.0), Some(button));
}
