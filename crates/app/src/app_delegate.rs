//! AppDelegate —— objc2 0.5.x 的 declare_class! 老语法定义。

use std::cell::RefCell;

use agent_light_core::{Monitor, Snapshot};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2::{class, declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSStatusItem, NSWindow};
use objc2_foundation::NSObjectProtocol;

use crate::overlay::PillView;
use crate::panel::Popover;

/// AppDelegate 的实例变量(方法只能拿 &self,故用 RefCell)。
pub struct AppIvars {
    pub monitor: Monitor,
    pub status_item: RefCell<Option<Retained<NSStatusItem>>>,
    /// 浮窗窗口;保活 + 切换点击穿透时读。
    pub overlay_window: RefCell<Option<Retained<NSWindow>>>,
    pub overlay_view: RefCell<Option<Retained<PillView>>>,
    pub popover: RefCell<Option<Popover>>,
    /// 设置窗;首次打开时懒创建。
    pub settings_window: RefCell<Option<Retained<NSWindow>>>,
    /// 浮窗是否点击穿透。true=穿透(默认);false=接收鼠标可拖动。
    pub click_through: RefCell<bool>,
    /// 上一轮的状态签名;相同则跳过渲染(省 CPU)。
    pub last_sig: RefCell<String>,
}

declare_class!(
    pub struct AppDelegate;

    unsafe impl ClassType for AppDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "AppDelegate";
    }

    impl DeclaredClass for AppDelegate {
        type Ivars = AppIvars;
    }

    #[allow(non_snake_case)]
    unsafe impl AppDelegate {
        /// NSTimer 每 ~2s:轮询内核 → 状态有变化才渲染(菜单栏灯 + 浮窗 + popover)。
        #[method(tick:)]
        fn tick(&self, _timer: *mut NSObject) {
            let snap = self.ivars().monitor.poll();
            let sig = signature(&snap);
            let same = { let last = self.ivars().last_sig.borrow(); *last == sig };
            if same {
                return;
            }
            *self.ivars().last_sig.borrow_mut() = sig;
            self.render(&snap);
        }

        /// 点状态栏 Asig:弹出/收起详情 popover。首次点击时懒创建窗口。
        #[method(togglePopover:)]
        fn toggle_popover(&self, _sender: *mut NSObject) {
            if self.ivars().popover.borrow().is_none() {
                let p = crate::panel::build(self);
                *self.ivars().popover.borrow_mut() = Some(p);
            }
            if let Some(p) = self.ivars().popover.borrow().as_ref() {
                crate::panel::toggle(p);
            }
        }

        /// popover 里"设置…"按钮:打开设置窗口。首次打开时懒创建。
        #[method(openSettings:)]
        fn open_settings(&self, _sender: *mut NSObject) {
            if self.ivars().settings_window.borrow().is_none() {
                let w = crate::settings::build(self);
                *self.ivars().settings_window.borrow_mut() = Some(w);
            }
            if let Some(w) = self.ivars().settings_window.borrow().as_ref() {
                crate::settings::show(w);
            }
        }

        /// "退出"按钮 / 菜单 action。
        #[method(quit:)]
        fn quit(&self, _sender: *mut NSObject) {
            let app: Retained<NSApplication> =
                unsafe { msg_send_id![class!(NSApplication), sharedApplication] };
            let _: () = unsafe { msg_send![&app, terminate: std::ptr::null_mut::<NSObject>()] };
        }

        /// 设置面板「浮窗点击穿透」复选框 action。sender=复选框,读其 state。
        #[method(toggleClickThrough:)]
        fn toggle_click_through(&self, sender: *mut NSObject) {
            let state: i64 = unsafe { msg_send![sender, state] }; // NSOnState=1 / NSOffState=0
            let on = state == 1;
            *self.ivars().click_through.borrow_mut() = on;
            self.apply_click_through();
        }
    }
);

impl AppDelegate {
    /// 把 click_through 设置同步到浮窗窗口。
    fn apply_click_through(&self) {
        let on = *self.ivars().click_through.borrow();
        if let Some(w) = self.ivars().overlay_window.borrow().as_ref() {
            crate::overlay::set_click_through(w, on);
        }
    }
}

impl AppDelegate {
    /// 把快照渲染到所有 UI(菜单栏灯 + 浮窗 + popover)。
    fn render(&self, snap: &Snapshot) {
        if let Some(item) = self.ivars().status_item.borrow().as_ref() {
            crate::tray::set_light(item, snap.global, snap.done_notif);
        }
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_light(view, snap.light());
        }
        if let Some(p) = self.ivars().popover.borrow().as_ref() {
            crate::panel::update_label(p, snap);
        }
    }
}

/// 状态签名:全局态 + done_notif + 各会话(id + status)。相同则视为无变化,跳过渲染。
fn signature(snap: &Snapshot) -> String {
    let mut s = format!("{:?}|{}|", snap.global, snap.done_notif);
    for sess in &snap.sessions {
        s.push_str(&format!("{}:{:?};", sess.id, sess.status));
    }
    s
}

// declare_class 不会自动 impl NSObjectProtocol;NSApplicationDelegate 要求它。
unsafe impl NSObjectProtocol for AppDelegate {}
unsafe impl NSApplicationDelegate for AppDelegate {}

// 普通 Rust 构造器(非 ObjC 方法):alloc → set_ivars → super init。
impl AppDelegate {
    pub fn new(ivars: AppIvars) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send_id![Self::class(), alloc] };
        let partial = allocated.set_ivars(ivars);
        unsafe { msg_send_id![super(partial), init] }
    }
}
