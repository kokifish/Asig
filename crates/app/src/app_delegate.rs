//! AppDelegate —— objc2 0.6 的 define_class! 定义。

use std::cell::RefCell;

use agent_light_core::{
    AgentStatus, Anim, Color, Lang, LightAnim, LightPosition, Monitor, Settings, Snapshot,
    StyleKey, Theme,
};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationDelegate, NSStatusItem, NSView, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSString, NSTimer};
use std::collections::HashMap;

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
    /// 侧栏选中药丸(液态玻璃,共享一个);update_selection 按选中 tab 移位/显隐。
    pub settings_selection: RefCell<Option<Retained<NSView>>>,
    /// 各状态 pane 的控件(色块/radio/速度),按 StyleKey 索引;reset / 选择变更时刷新。
    pub state_controls: RefCell<HashMap<StyleKey, crate::settings::StateControls>>,
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
            // dev 预览(ASIG_PREVIEW=1):跳过轮询,循环展示各状态默认灯效。
            if std::env::var_os("ASIG_PREVIEW").is_some() {
                return self.preview_tick();
            }
            self.persist_light_pos();
            let snap = self.snap();
            // 把 Reduce Motion 并入签名:用户在系统设置里切该开关时,签名变化 → 立即重渲染,
            // set_light 据 reduce_motion_on 把动画降级为常亮(无需常驻渲染,不损 CPU)。
            // 签名并入 reduce_motion + 外观(app):系统深浅 / Theme 切换时签名变化 → 下次
            // tick 重绘(浮窗自绘 drawRect 已实时适配;菜单栏/色块借此 ≤ 轮询周期内刷新)。
            let sig = format!(
                "{}|rm={}|app={}",
                signature(&snap),
                crate::overlay::reduce_motion_on(),
                crate::overlay::is_dark_appearance()
            );
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
            let mtm = MainThreadMarker::new().expect("togglePopover 须主线程");
            let button = self
                .ivars()
                .status_item
                .borrow()
                .as_ref()
                .and_then(|item| item.button(mtm));
            // 右键 → 状态栏菜单;左键 → 下拉 popover
            let app: Retained<NSApplication> =
                unsafe { msg_send![class!(NSApplication), sharedApplication] };
            let is_right = unsafe {
                app.currentEvent()
                    .map(|ev| -> bool {
                        let et: i64 = msg_send![&ev, type];
                        et == 3
                    })
                    .unwrap_or(false)
            };
            if is_right {
                if let Some(button) = button {
                    crate::tray::show_status_menu(self, &button, mtm);
                }
                return;
            }
            let p = crate::panel::build(self);
            if let Some(button) = button {
                crate::panel::show(&p, &button);
            }
            *self.ivars().popover.borrow_mut() = Some(p);
            let snap = self.snap();
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

        /// 设置面板「浮窗灯大小」滑块 action;同步刷新右侧 `xx px` 标签。
        #[unsafe(method(changeSize:))]
        fn change_size(&self, sender: *mut NSObject) {
            let v: f64 = unsafe { msg_send![sender, doubleValue] };
            let dot = v.round().max(6.0) as u32;
            self.ivars().settings.borrow_mut().dot_size = dot;
            if let Some(content) = self.ivars().settings_content.borrow().as_ref() {
                if let Some(label) =
                    crate::settings::view_with_tag(content, crate::settings::SIZE_LABEL_TAG)
                {
                    unsafe {
                        let _: () = msg_send![
                            &label,
                            setStringValue: &*NSString::from_str(&format!("{} px", dot))
                        ];
                    }
                }
            }
            self.settings_changed();
        }

        /// 状态 pane「Color」色块单选 action。tag = base + COLOR_OFF + i。
        #[unsafe(method(changeColor:))]
        fn change_color(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let Some((key, sub)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            let i = (sub - crate::settings::COLOR_OFF) as usize;
            if i >= crate::settings::COLOR_ORDER.len() {
                return;
            }
            {
                let mut s = self.ivars().settings.borrow_mut();
                s.styles.entry(key).or_insert(key.default_style()).color =
                    crate::settings::COLOR_ORDER[i];
            }
            self.refresh_state(key);
            self.settings_changed();
        }

        /// 状态 pane「Animation」单选 action。tag = base + ANIM_OFF + i。
        #[unsafe(method(changeAnim:))]
        fn change_anim(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let Some((key, sub)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            let i = (sub - crate::settings::ANIM_OFF) as usize;
            if i >= crate::settings::ANIM_ORDER.len() {
                return;
            }
            {
                let mut s = self.ivars().settings.borrow_mut();
                let st = s.styles.entry(key).or_insert(key.default_style());
                st.anim = crate::settings::ANIM_ORDER[i];
                if st.anim != Anim::Steady && st.period_ms == 0 {
                    st.period_ms = 1000; // 离开常亮时给个默认周期
                }
            }
            self.refresh_state(key);
            self.settings_changed();
        }

        /// 状态 pane「Speed」滑块 action(Hz)。tag = base + SPEED_OFF。
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
            if let Some(c) = self.ivars().state_controls.borrow().get(&key) {
                unsafe {
                    let _: () = msg_send![
                        &c.speed_label,
                        setStringValue: &*NSString::from_str(&format!("{:.1} Hz", hz))
                    ];
                }
            }
            self.settings_changed();
        }

        /// DoneNotif pane「持续时间」滑块 action(秒)。改完存盘;下一轮 tick 按新时长判窗口。
        #[unsafe(method(changeDuration:))]
        fn change_duration(&self, sender: *mut NSObject) {
            let v: f64 = unsafe { msg_send![sender, doubleValue] };
            let secs = v.round().clamp(
                agent_light_core::DONE_NOTIF_DURATION_MIN_S as f64,
                agent_light_core::DONE_NOTIF_DURATION_MAX_S as f64,
            ) as u32;
            self.ivars().settings.borrow_mut().done_notif_duration_s = secs;
            if let Some(c) = self
                .ivars()
                .state_controls
                .borrow()
                .get(&StyleKey::DoneNotif)
            {
                crate::settings::refresh_duration(c, secs);
            }
            self.settings_changed();
        }

        /// 状态 pane「Reset」action:恢复该状态默认样式并刷新控件。
        #[unsafe(method(resetStateStyle:))]
        fn reset_state(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let Some((key, _)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            {
                let mut s = self.ivars().settings.borrow_mut();
                s.styles.insert(key, key.default_style());
                // DoneNotif 的「持续时间」也是该状态配置,reset 一并回默认。
                if key == StyleKey::DoneNotif {
                    s.done_notif_duration_s = agent_light_core::DONE_NOTIF_DURATION_DEFAULT_S;
                }
            }
            self.refresh_state(key);
            self.settings_changed();
        }

        /// General「Language」单选 action。tag = LANG_EN_TAG / LANG_ZH_TAG。切换后重建设置窗。
        #[unsafe(method(changeLanguage:))]
        fn change_language(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let lang = if tag == crate::settings::LANG_EN_TAG {
                Lang::En
            } else {
                Lang::Zh
            };
            self.ivars().settings.borrow_mut().lang = lang;
            self.ivars().settings.borrow().save();
            self.rebuild_settings();
        }

        /// General「Reset 全部」action:确认对话框 → 重置所有自定义(语言 + 各状态)→ 重应用 + 重建。
        #[unsafe(method(resetAll:))]
        fn reset_all(&self, _sender: *mut NSObject) {
            let lang = self.ivars().settings.borrow().lang;
            let (title, msg, yes, no) = crate::settings::reset_confirm_texts(lang);
            let alert: Retained<NSAlert> = unsafe { msg_send![class!(NSAlert), new] };
            unsafe {
                let _: () = msg_send![&alert, setMessageText: &*NSString::from_str(title)];
                let _: () = msg_send![&alert, setInformativeText: &*NSString::from_str(msg)];
                let _: () = msg_send![&alert, addButtonWithTitle: &*NSString::from_str(yes)];
                let _: () = msg_send![&alert, addButtonWithTitle: &*NSString::from_str(no)];
            }
            let resp: i64 = unsafe { msg_send![&alert, runModal] };
            if resp != 1000 {
                return; // NSAlertFirstButtonReturn = 1000;非「重置」则取消
            }
            // 重置全部自定义
            *self.ivars().settings.borrow_mut() = Settings::default();
            self.ivars().settings.borrow().save();
            *self.ivars().click_through.borrow_mut() = true;
            // 重应用:浮窗大小 + 点击穿透 + tick 重排
            let dot = self.ivars().settings.borrow().dot_size;
            if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
                crate::overlay::set_size(view, dot);
            }
            self.apply_click_through();
            let ms = self.ivars().settings.borrow().poll_interval_ms;
            crate::tray::reschedule(self, ms as f64 / 1000.0);
            let snap = self.snap();
            self.render(&snap);
            self.rebuild_settings();
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
            crate::settings::update_selection(self, new);
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

        /// General「Theme」radio action。sender tag − THEME_OFF = 0/1/2 = 跟随系统/深/浅。
        /// 设 NSApp.appearance + 存盘 + 重建(radio 选中态据新 theme 重设)+ 重绘。
        #[unsafe(method(changeTheme:))]
        fn change_theme(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let theme = match tag - crate::settings::THEME_OFF {
                1 => Theme::Dark,
                2 => Theme::Light,
                _ => Theme::FollowSystem,
            };
            self.ivars().settings.borrow_mut().theme = theme;
            self.ivars().settings.borrow().save();
            crate::overlay::apply_theme(theme);
            self.rebuild_settings();
            let snap = self.snap();
            self.render(&snap);
        }

        /// 占位 action(禁用的「开机启动」等无操作控件的兜底,实际不会触发)。
        #[unsafe(method(noop:))]
        fn noop(&self, _sender: *mut NSObject) {}

        /// Settings 窗口尺寸变化:按右区新宽度重排所有 state pane 的色块
        /// (固定间距 flow——宽度变时自动换行 / 很宽时合并为 1 行,色块间距恒定;
        /// card 高度也随之按行数重算)。其余 pane 靠 autoresizing 自适应宽度。
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notif: *mut NSObject) {
            let pane_w = self
                .ivars()
                .settings_content
                .borrow()
                .as_ref()
                .map(|c| {
                    let f: NSRect = unsafe { msg_send![&**c, frame] };
                    f.size.width
                })
                .filter(|w| *w > 0.0)
                .unwrap_or(crate::settings::CONTENT_W);
            let controls = self.ivars().state_controls.borrow();
            for c in controls.values() {
                crate::settings::layout_state_pane(c, pane_w);
            }
        }
    }

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {}

    unsafe impl NSWindowDelegate for AppDelegate {}
);

impl AppDelegate {
    /// 把 click_through 设置同步到浮窗窗口。
    fn apply_click_through(&self) {
        let on = *self.ivars().click_through.borrow();
        if let Some(w) = self.ivars().overlay_window.borrow().as_ref() {
            crate::overlay::set_click_through(w, on);
        }
    }

    /// 用某状态当前 settings 刷新其 pane 控件(色块选中环 / radio / 速度滑块+标签)。
    fn refresh_state(&self, key: StyleKey) {
        let style = self.ivars().settings.borrow().style_for(key);
        let controls = self.ivars().state_controls.borrow();
        if let Some(c) = controls.get(&key) {
            crate::settings::refresh_state_controls(c, style);
            // DoneNotif 的持续时间滑块/标签也随刷新(reset 后回默认)。
            if key == StyleKey::DoneNotif {
                let secs = self.ivars().settings.borrow().done_notif_duration_s;
                crate::settings::refresh_duration(c, secs);
            }
        }
    }

    /// 关闭旧设置窗、丢弃其 pane/控件引用,按当前(可能已变的语言/设置)重新构建并显示。
    fn rebuild_settings(&self) {
        if let Some(w) = self.ivars().settings_window.borrow_mut().take() {
            let _: () = unsafe { msg_send![&w, close] };
        }
        *self.ivars().settings_panes.borrow_mut() = None;
        *self.ivars().settings_sidebar.borrow_mut() = None;
        *self.ivars().settings_content.borrow_mut() = None;
        *self.ivars().settings_selected.borrow_mut() = 0;
        self.ivars().state_controls.borrow_mut().clear();
        let w = crate::settings::build(self);
        *self.ivars().settings_window.borrow_mut() = Some(w);
        if let Some(w) = self.ivars().settings_window.borrow().as_ref() {
            crate::settings::show(w);
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

    /// 取一次快照:把 settings 里的 DoneNotif 持续时间 clamp 到合法范围后喂给内核 poll。
    /// 内核 poll 不持有用户设置(保持纯净),故时长由 app 层每次喂入。
    fn snap(&self) -> Snapshot {
        let secs = self.ivars().settings.borrow().done_notif_duration_s.clamp(
            agent_light_core::DONE_NOTIF_DURATION_MIN_S,
            agent_light_core::DONE_NOTIF_DURATION_MAX_S,
        );
        self.ivars()
            .monitor
            .poll(std::time::Duration::from_secs(secs as u64))
    }

    /// 设置改动后:存盘 + 立即重应用(圆点大小 + 灯效),不等下一轮 tick。
    fn settings_changed(&self) {
        self.ivars().settings.borrow().save();
        let dot = self.ivars().settings.borrow().dot_size;
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_size(view, dot);
        }
        let snap = self.snap();
        self.render(&snap);
    }

    /// dev 预览(ASIG_PREVIEW=1):不轮询,每个 tick(~3s)把浮窗灯切到下一状态的**默认**动效并打印,
    /// 便于一行命令查看 Done/DoneNotif/Working/NeedsDeci/Error/Offline 的默认灯效。循环不息。
    fn preview_tick(&self) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static IDX: AtomicUsize = AtomicUsize::new(0);
        // (名称, 默认动效)。DoneNotif 不是 AgentStatus,单独构造其浅蓝快呼吸。
        let states: [(&str, LightAnim); 6] = [
            ("Done", AgentStatus::Done.light()),
            (
                "DoneNotif",
                LightAnim::Pulse {
                    color: Color::LightBlue,
                    period_ms: 450,
                },
            ),
            ("Working", AgentStatus::Working.light()),
            ("NeedsDeci", AgentStatus::NeedsDeci.light()),
            ("Error", AgentStatus::Error.light()),
            ("Offline", AgentStatus::Offline.light()),
        ];
        let (name, anim) = states[IDX.fetch_add(1, Ordering::SeqCst) % states.len()];
        let mtm = MainThreadMarker::new().expect("preview 须在主线程");
        if let Some(item) = self.ivars().status_item.borrow().as_ref() {
            crate::tray::set_light(item, &anim, mtm);
        }
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_light(view, anim);
        }
        println!("[asig-preview] {name}: {anim:?}");
        let mut out = std::io::stdout();
        let _ = std::io::Write::flush(&mut out);
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
