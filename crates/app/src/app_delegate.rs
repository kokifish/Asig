//! AppDelegate —— objc2 0.6 的 define_class! 定义。

use std::cell::RefCell;

use agent_light_core::{LightPosition, Monitor, Settings, Snapshot, StyleKey};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSStatusItem, NSWindow};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect};

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
    /// 用户设置(灯大小 + 各状态样式);启动加载,改动即存盘。
    pub settings: RefCell<Settings>,
    /// 上一轮的状态签名;相同则跳过渲染(省 CPU)。
    pub last_sig: RefCell<String>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppDelegate"]
    #[ivars = AppIvars]
    pub struct AppDelegate;

    #[allow(non_snake_case)]
    impl AppDelegate {
        /// NSTimer 每 ~3s:轮询内核 → 状态有变化才渲染(菜单栏灯 + 浮窗 + popover)。
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: *mut NSObject) {
            self.persist_light_pos();
            let snap = self.ivars().monitor.poll();
            let sig = signature(&snap);
            let same = {
                let last = self.ivars().last_sig.borrow();
                *last == sig
            };
            if same {
                return;
            }
            *self.ivars().last_sig.borrow_mut() = sig;
            self.render(&snap);
        }

        /// 单击菜单栏 Signal Icon:弹/收 Drop-down Panel。位置按图标算;隐藏即丢弃,
        /// 下次显示重建(拿最新位置 + 锁定态 + 不占常驻内存)。
        #[unsafe(method(togglePopover:))]
        fn toggle_popover(&self, _sender: *mut NSObject) {
            let visible = self
                .ivars()
                .popover
                .borrow()
                .as_ref()
                .map(crate::panel::is_visible)
                .unwrap_or(false);
            if visible {
                if let Some(p) = self.ivars().popover.borrow().as_ref() {
                    crate::panel::hide(p);
                }
                *self.ivars().popover.borrow_mut() = None;
                return;
            }
            let pos = self.ivars().status_item.borrow().as_ref().map(|item| {
                crate::panel::dropdown_position_for(
                    item,
                    crate::panel::PANEL_W,
                    crate::panel::PANEL_H,
                )
            });
            let p = crate::panel::build(self, pos);
            crate::panel::show(&p);
            *self.ivars().popover.borrow_mut() = Some(p);
            let snap = self.ivars().monitor.poll();
            self.render(&snap);
        }

        /// popover 里"设置…"按钮:打开设置窗口。首次打开时懒创建。
        #[unsafe(method(openSettings:))]
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
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: *mut NSObject) {
            let app: Retained<NSApplication> =
                unsafe { msg_send![class!(NSApplication), sharedApplication] };
            let _: () = unsafe { msg_send![&app, terminate: std::ptr::null_mut::<NSObject>()] };
        }

        /// 设置面板「浮窗点击穿透」复选框 action。sender=复选框,读其 state。
        #[unsafe(method(toggleClickThrough:))]
        fn toggle_click_through(&self, sender: *mut NSObject) {
            let state: i64 = unsafe { msg_send![sender, state] }; // NSOnState=1 / NSOffState=0
            let on = state == 1;
            *self.ivars().click_through.borrow_mut() = on;
            self.apply_click_through();
        }

        /// 设置面板「大小」滑块 action。
        #[unsafe(method(changeSize:))]
        fn change_size(&self, sender: *mut NSObject) {
            let v: f64 = unsafe { msg_send![sender, doubleValue] };
            self.ivars().settings.borrow_mut().dot_size = v.round().max(6.0) as u32;
            self.settings_changed();
        }

        /// 设置面板「样式」下拉 action。tag = key_idx*2 + field(0=动画,1=颜色)。
        /// key_idx 索引 `StyleKey::ALL`(5 状态 + Done-Notification)。
        #[unsafe(method(changeStyle:))]
        fn change_style(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let idx: i64 = unsafe { msg_send![sender, indexOfSelectedItem] };
            let key_idx = (tag / 2) as usize;
            let field = tag % 2;
            let Some(key) = StyleKey::ALL.get(key_idx).copied() else {
                return;
            };
            let mut settings = self.ivars().settings.borrow_mut();
            let st = settings.styles.entry(key).or_insert(key.default_style());
            match field {
                0 => st.anim = crate::settings::ANIM_ORDER[idx as usize],
                1 => st.color = crate::settings::COLOR_ORDER[idx as usize],
                _ => {}
            }
            drop(settings);
            self.settings_changed();
        }
    }

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {}
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
    /// 把快照渲染到所有 UI(菜单栏灯 + 浮窗 + popover)。灯效来自用户设置。
    fn render(&self, snap: &Snapshot) {
        let anim = self.ivars().settings.borrow().light(snap);
        // 渲染总在主线程(tick / 点击 / 设置改动均主线程触发);button() 要 MainThreadMarker。
        let mtm = MainThreadMarker::new().expect("render 须在主线程");
        if let Some(item) = self.ivars().status_item.borrow().as_ref() {
            crate::tray::set_light(item, &anim, mtm);
        }
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_light(view, anim);
        }
        if let Some(p) = self.ivars().popover.borrow().as_ref() {
            crate::panel::update_label(p, snap);
        }
    }

    /// 设置改动后:存盘 + 立即重应用(圆点大小 + 灯效),不等下一轮 tick。
    fn settings_changed(&self) {
        self.ivars().settings.borrow().save();
        let dot = self.ivars().settings.borrow().dot_size;
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_size(view, dot);
        }
        let snap = self.ivars().monitor.poll();
        self.render(&snap);
    }

    /// 记住浮窗当前位置(全局 origin + 所在屏 id),供下次启动恢复。tick 每 ~3s 调一次,
    /// 仅在位置变化时写盘 —— 比 windowDidMove 更省事,且抗强杀(3s 内必落盘)。
    fn persist_light_pos(&self) {
        let frame = {
            let win = self.ivars().overlay_window.borrow();
            let Some(w) = win.as_ref() else { return };
            let f: NSRect = unsafe { msg_send![&**w, frame] };
            f
        };
        let center = NSPoint::new(
            frame.origin.x + frame.size.width / 2.0,
            frame.origin.y + frame.size.height / 2.0,
        );
        let pos = LightPosition {
            x: frame.origin.x,
            y: frame.origin.y,
            screen_id: crate::overlay::screen_id_at(center),
        };
        let mut s = self.ivars().settings.borrow_mut();
        if s.light_pos != Some(pos) {
            s.light_pos = Some(pos);
            drop(s);
            self.ivars().settings.borrow().save();
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

// 普通 Rust 构造器(非 ObjC 方法):alloc → set_ivars → super init。
impl AppDelegate {
    pub fn new(ivars: AppIvars) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send![Self::class(), alloc] };
        let partial = allocated.set_ivars(ivars);
        unsafe { msg_send![super(partial), init] }
    }
}
