//! AppDelegate —— objc2 0.6 的 define_class! 定义。

use std::cell::RefCell;

use agent_light_core::{Anim, LightPosition, Monitor, Settings, Snapshot, StyleKey};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSStatusItem, NSView, NSWindow};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSString, NSTimer};

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
    /// tick 定时器引用;轮询间隔改动时作废旧 timer 重排。
    pub tick_timer: RefCell<Option<Retained<NSTimer>>>,
    /// 设置窗侧栏(切换 tab 时改前缀用)。
    pub settings_sidebar: RefCell<Option<Retained<NSView>>>,
    /// 设置窗右侧内容区(viewWithTag 找控件用)。
    pub settings_content: RefCell<Option<Retained<NSView>>>,
    /// 设置窗 8 个 pane(按 pane id 0..7 排列:常规/DoneNotif/.../Offline/关于)。切 tab 用。
    pub settings_panes: RefCell<Option<Vec<Retained<NSView>>>>,
    /// 设置窗当前选中的 tab(pane id)。
    pub settings_selected: RefCell<i64>,
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

        /// 状态 pane 的「颜色 / 动画」下拉 action。tag 编码 (state, field)(见 settings.rs)。
        #[unsafe(method(changeStyle:))]
        fn change_style(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let idx: i64 = unsafe { msg_send![sender, indexOfSelectedItem] };
            let Some((key, field)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            let new_anim = {
                let mut s = self.ivars().settings.borrow_mut();
                let st = s.styles.entry(key).or_insert(key.default_style());
                match field {
                    crate::settings::F_COLOR => {
                        st.color = crate::settings::COLOR_ORDER[idx as usize];
                    }
                    crate::settings::F_ANIM => {
                        st.anim = crate::settings::ANIM_ORDER[idx as usize];
                        if st.anim != Anim::Steady && st.period_ms == 0 {
                            st.period_ms = 1000; // 离开常亮时给个默认周期
                        }
                    }
                    _ => {}
                }
                st.anim
            };
            // 动画变了 → 同步速度滑块启用态 + 标签(常亮则禁用并显示「—」)
            if field == crate::settings::F_ANIM {
                self.sync_speed_controls(tag, key, new_anim);
            }
            self.settings_changed();
        }

        /// 侧栏 tab / 关于图标点击:切换右侧 pane。tag = pane id(0=常规 … 7=关于)。
        #[unsafe(method(switchSettingsTab:))]
        fn switch_settings_tab(&self, sender: *mut NSObject) {
            let new: i64 = unsafe { msg_send![sender, tag] };
            let old = *self.ivars().settings_selected.borrow();
            if old == new || !(0..8).contains(&new) {
                return;
            }
            let panes = self.ivars().settings_panes.borrow();
            if let Some(v) = panes.as_ref() {
                if let Some(p) = v.get(old as usize) {
                    let _: () = unsafe { msg_send![p, setHidden: Bool::YES] };
                }
                if let Some(p) = v.get(new as usize) {
                    let _: () = unsafe { msg_send![p, setHidden: Bool::NO] };
                }
            }
            *self.ivars().settings_selected.borrow_mut() = new;
            crate::settings::update_tab_prefixes(self, new);
        }

        /// 常规页「轮询间隔」下拉 action。改完即时重排 tick 定时器。
        #[unsafe(method(changePollInterval:))]
        fn change_poll_interval(&self, sender: *mut NSObject) {
            let idx: i64 = unsafe { msg_send![sender, indexOfSelectedItem] };
            let Some(&ms) = crate::settings::POLL_PRESETS_MS.get(idx as usize) else {
                return;
            };
            self.ivars().settings.borrow_mut().poll_interval_ms = ms;
            self.settings_changed();
            crate::tray::reschedule(self, ms as f64 / 1000.0);
        }

        /// 状态 pane「速度」滑块 action(Hz)。tag 编码 (state, F_SPEED);实时刷新标签。
        #[unsafe(method(changeSpeed:))]
        fn change_speed(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let Some((key, _)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            let hz: f64 = unsafe { msg_send![sender, doubleValue] };
            let period_ms = (1000.0 / hz).round().max(1.0) as u32;
            {
                let mut s = self.ivars().settings.borrow_mut();
                s.styles.entry(key).or_insert(key.default_style()).period_ms = period_ms;
            }
            if let Some(content) = self.ivars().settings_content.borrow().as_ref().cloned() {
                let label_tag = (tag / 100) * 100 + crate::settings::F_SPEED_LABEL;
                if let Some(lbl) = crate::settings::view_with_tag(&content, label_tag) {
                    unsafe {
                        let _: () = msg_send![
                            &lbl,
                            setStringValue: &*NSString::from_str(&format!("{:.1} Hz", hz))
                        ];
                    }
                }
            }
            self.settings_changed();
        }

        /// 占位 action(禁用的「开机启动」等无操作控件的兜底,实际不会触发)。
        #[unsafe(method(noop:))]
        fn noop(&self, _sender: *mut NSObject) {}
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

    /// 动画切换后,按 tag 找到该状态的速度滑块 + 标签:常亮→禁用并显示「—」,否则启用。
    fn sync_speed_controls(&self, control_tag: i64, key: StyleKey, anim: Anim) {
        let Some(content) = self.ivars().settings_content.borrow().as_ref().cloned() else {
            return;
        };
        let base = (control_tag / 100) * 100;
        let steady = anim == Anim::Steady;
        if let Some(slider) =
            crate::settings::view_with_tag(&content, base + crate::settings::F_SPEED)
        {
            unsafe {
                let _: () = msg_send![&slider, setEnabled: Bool::new(!steady)];
            }
        }
        if let Some(lbl) =
            crate::settings::view_with_tag(&content, base + crate::settings::F_SPEED_LABEL)
        {
            let text = if steady {
                "—".to_string()
            } else {
                let p = self
                    .ivars()
                    .settings
                    .borrow()
                    .style_for(key)
                    .period_ms
                    .max(1);
                format!("{:.1} Hz", 1000.0 / p as f64)
            };
            unsafe {
                let _: () = msg_send![&lbl, setStringValue: &*NSString::from_str(&text)];
            }
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
