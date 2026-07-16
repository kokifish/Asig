//! AppDelegate —— objc2 0.6 的 define_class! 定义。

use std::cell::RefCell;

use agent_light_core::{
    AgentStatus, Anim, Color, GRADIENT_LAYERS_DEFAULT, GRADIENT_LAYERS_MAX, GRADIENT_LAYERS_MIN,
    Lang, LightAnim, LightPosition, Monitor, Settings, Snapshot, StateStyle, StyleKey, Theme,
};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationDelegate, NSEventType, NSScrollView, NSStatusItem, NSView,
    NSWindow, NSWindowDelegate,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSString, NSTimer};
use std::collections::HashMap;

use crate::overlay::PillView;
use crate::panel::Popover;

/// AppDelegate 的实例变量(方法只能拿 &self,故用 RefCell)。
pub struct AppIvars {
    pub monitor: RefCell<Monitor>,
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
    /// 设置窗右侧内容区(viewWithTag 找控件用;存 scrollView 的 NSView 视图)。
    pub settings_content: RefCell<Option<Retained<NSView>>>,
    /// 设置窗右区 scrollView(切 pane 时读 documentView 设高 + 滚顶)。
    pub settings_scroll: RefCell<Option<Retained<NSScrollView>>>,
    /// 各 pane(按 tab id 0..7)的实际内容高;切 pane 时据此设 documentView 高。
    pub settings_pane_heights: RefCell<HashMap<i64, CGFloat>>,
    /// 设置窗 8 个 pane(按 pane id 0..7 排列:常规/DoneNotif/.../Offline/关于)。切 tab 用。
    pub settings_panes: RefCell<Option<Vec<Retained<NSView>>>>,
    /// 设置窗当前选中的 tab(pane id)。
    pub settings_selected: RefCell<i64>,
    /// 侧栏选中药丸(液态玻璃,共享一个);update_selection 按选中 tab 移位/显隐。
    pub settings_selection: RefCell<Option<Retained<NSView>>>,
    /// 各状态 pane 的控件(色块/radio/速度),按 StyleKey 索引;reset / 选择变更时刷新。
    pub state_controls: RefCell<HashMap<StyleKey, crate::settings::StateControls>>,
    /// 上一轮的全局状态;转入时触发系统通知(边沿检测)。
    pub last_global: RefCell<Option<AgentStatus>>,
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
            self.maybe_notify(&snap);
            // 把 Reduce Motion 并入签名:用户在系统设置里切该开关时,签名变化 → 立即重渲染,
            // set_light 据 reduce_motion_on 把动画降级为常亮(无需常驻渲染,不损 CPU)。
            // 签名并入 reduce_motion + 外观(app):系统深浅 / Theme 切换时签名变化 → 下次
            // tick 重绘(浮窗自绘 drawRect 已实时适配;菜单栏/色块借此 ≤ 轮询周期内刷新)。
            let sig = format!(
                "{}|rm={}|app={}",
                snap.signature(),
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
            let app = NSApplication::sharedApplication(mtm);
            let is_right = app
                .currentEvent()
                .map(|ev| ev.r#type() == NSEventType::RightMouseDown)
                .unwrap_or(false);
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
            let mtm = MainThreadMarker::new().expect("quit 须在主线程");
            let app = NSApplication::sharedApplication(mtm);
            app.terminate(None);
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
            let dot = v.round().clamp(
                agent_light_core::DOT_SIZE_MIN_PX as f64,
                agent_light_core::DOT_SIZE_MAX_PX as f64,
            ) as u32;
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
            let color = crate::settings::COLOR_ORDER[i];
            self.edit_style(key, |st| st.color = color);
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
            let anim = crate::settings::ANIM_ORDER[i];
            self.edit_style(key, |st| {
                st.anim = anim;
                if st.anim != Anim::Steady && st.period_ms == 0 {
                    st.period_ms = 1000; // 离开常亮时给个默认周期
                }
            });
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
            self.edit_style(key, |st| st.period_ms = period_ms);
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

        /// 状态 pane「渐变层数」滑块 action(整数 0..=4)。tag = base + GRADIENT_OFF。
        /// 仅作用于浮窗圆点本体;改完存盘 + 重渲染(set_light → drawRect 据 layers 画同心环)。
        #[unsafe(method(changeGradient:))]
        fn change_gradient(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let Some((key, _)) = crate::settings::parse_control_tag(tag) else {
                return;
            };
            let v: f64 = unsafe { msg_send![sender, doubleValue] };
            let layers = v
                .round()
                .clamp(GRADIENT_LAYERS_MIN as f64, GRADIENT_LAYERS_MAX as f64)
                as u8;
            self.edit_style(key, |st| st.gradient_layers = layers);
            if let Some(c) = self.ivars().state_controls.borrow().get(&key) {
                unsafe {
                    let _: () = msg_send![
                        &c.gradient_label,
                        setStringValue: &*NSString::from_str(&format!("{}", layers))
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
            let mtm = MainThreadMarker::new().expect("resetAll 须在主线程");
            let lang = self.ivars().settings.borrow().lang;
            let (title, msg, yes, no) = crate::settings::reset_confirm_texts(lang);
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(title));
            alert.setInformativeText(&NSString::from_str(msg));
            alert.addButtonWithTitle(&NSString::from_str(yes));
            alert.addButtonWithTitle(&NSString::from_str(no));
            let resp = alert.runModal();
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
            // documentView 高度 = max(clip 可视高, 新 pane content_h),并滚到顶(避免残留上一
            // pane 的滚动位置)。取 max 而非纯 content_h:doc 矮于 clip 时 NSClipView 对翻转短文档
            // 的顶部锚定随 doc 高漂移,致各 pane 内容顶部不对齐(见 settings::set_doc_height)。
            let new_h = self
                .ivars()
                .settings_pane_heights
                .borrow()
                .get(&new)
                .copied()
                .unwrap_or(crate::settings::H);
            if let Some(scroll) = self.ivars().settings_scroll.borrow().as_ref() {
                crate::settings::set_doc_height(scroll, new_h);
            }
            // 滚顶推到下一 runloop:doc.setFrameSize 触发 NSScrollView 的 layout pass
            // (reflectScrolledClippedView)会覆盖同步 setBoundsOrigin,performSelector afterDelay:0
            // 在 layout commit 后执行(见 scrollSettingsToTop:),根治切 tab 时 content 顶部漂移。
            let _: () = unsafe {
                msg_send![
                    self,
                    performSelector: sel!(scrollSettingsToTop:),
                    withObject: std::ptr::null_mut::<NSObject>(),
                    afterDelay: 0.0
                ]
            };
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

        /// General「监控的 Agent」多选 chip action(点击 toggle)。tag = AGENT_OFF + i →
        /// AGENT_KIND_ORDER[i]:已选→移除、未选→加入(允许全不选 = 不监控任何 agent)。
        /// 改完重建 Monitor(latched 清零)+ 刷新 chip 视觉 + 重渲染。
        #[unsafe(method(changeEnabledAgents:))]
        fn change_enabled_agents(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let i = (tag - crate::settings::AGENT_OFF) as usize;
            let Some(&kind) = crate::settings::AGENT_KIND_ORDER.get(i) else {
                return;
            };
            // recessed toggle button:点击后系统已切 state(on=1 监控 / off=0 不监控),据此改 Vec。
            let state: i64 = unsafe { msg_send![sender, state] };
            let mut kinds = self.ivars().settings.borrow().enabled_agents.clone();
            if state == 1 {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            } else {
                kinds.retain(|k| *k != kind);
            }
            self.ivars().settings.borrow_mut().enabled_agents = kinds.clone();
            // 先重建 Monitor(切走的 agent 的 latched 锁定态不应残留),再 settings_changed():
            // 后者 snap()+render() 才基于新 Monitor 画出切换后的真实状态;若反过来,首帧会
            // 用旧 Monitor(被取消的 agent 仍显示)直到下一轮 tick(~3s)。
            *self.ivars().monitor.borrow_mut() = agent_light_core::Monitor::with_enabled(&kinds);
            self.settings_changed();
        }

        /// General「状态通知」多选 chip action(点击 toggle)。tag = NOTIFY_OFF + i →
        /// NOTIFY_STATUS_ORDER[i]:已选→移除、未选→加入。改完仅存盘(无需重渲染:灯效不变,
        /// 只影响下次状态转入边沿时是否弹系统通知)。
        #[unsafe(method(changeNotifyOn:))]
        fn change_notify_on(&self, sender: *mut NSObject) {
            let tag: i64 = unsafe { msg_send![sender, tag] };
            let i = (tag - crate::settings::NOTIFY_OFF) as usize;
            let Some(&kind) = crate::settings::NOTIFY_STATUS_ORDER.get(i) else {
                return;
            };
            // recessed toggle button:点击后系统已切 state(on=1 通知 / off=0 不通知),据此改 Vec。
            let state: i64 = unsafe { msg_send![sender, state] };
            let mut kinds = self.ivars().settings.borrow().notify_on.clone();
            if state == 1 {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            } else {
                kinds.retain(|k| *k != kind);
            }
            self.ivars().settings.borrow_mut().notify_on = kinds;
            self.ivars().settings.borrow().save();
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

        /// 把设置窗右区内容滚到顶(clipView setBoundsOrigin=(0,0);flipped doc 下即顶部)。
        /// 单独成方法是为了让调用方用 performSelector:withObject:afterDelay:0 异步触发——
        /// 同步 setBoundsOrigin 会被 doc.setFrameSize 触发的 NSScrollView layout pass
        /// (reflectScrolledClippedView)覆盖,导致切 tab / 初始时 content 顶部漂移。afterDelay:0
        /// 推到下一轮 runloop,此时 layout 已 commit,setBoundsOrigin 稳得住,各 pane 顶部对齐。
        #[unsafe(method(scrollSettingsToTop:))]
        fn scroll_settings_to_top(&self, _sender: *mut NSObject) {
            if let Some(scroll) = self.ivars().settings_scroll.borrow().as_ref() {
                let cv = scroll.contentView();
                let _: () = unsafe { msg_send![&cv, setBoundsOrigin: NSPoint::new(0.0, 0.0)] };
            }
        }

        /// Settings 窗口尺寸变化:按右区 documentView 新宽度重排所有 state pane 的色块
        /// (固定间距 flow——宽度变时自动换行 / 很宽时合并为 1 行,色块间距恒定;
        /// card 高度也随之按行数重算)。其余 pane 靠 autoresizing 自适应宽度。
        /// pane 宽读 documentView(scrollView 的 doc)——其宽随 scrollView(autoresizing=2)。
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notif: *mut NSObject) {
            let pane_w = self
                .ivars()
                .settings_scroll
                .borrow()
                .as_ref()
                .and_then(|s| s.documentView())
                .map(|d| {
                    let f: NSRect = unsafe { msg_send![&d, frame] };
                    f.size.width
                })
                .filter(|w| *w > 0.0)
                .unwrap_or(crate::settings::CONTENT_W);
            let controls = self.ivars().state_controls.borrow();
            for c in controls.values() {
                crate::settings::layout_state_pane(c, pane_w);
            }
            // 窗口变高 → clip 可视高变大:重定 doc 高 = max(新 clip 高, 当前 pane content_h),
            // 否则原本填满 clip 的 doc 可能又矮于新 clip,重新触发短文档锚定漂移(见 set_doc_height)。
            let sel = *self.ivars().settings_selected.borrow();
            let ch = self
                .ivars()
                .settings_pane_heights
                .borrow()
                .get(&sel)
                .copied()
                .unwrap_or(crate::settings::H);
            if let Some(scroll) = self.ivars().settings_scroll.borrow().as_ref() {
                crate::settings::set_doc_height(scroll, ch);
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

    /// 编辑某状态样式(缺失键时按默认填补);借用 scope 内聚,refresh/save 由调用方决定。
    /// 收口 changeColor/Anim/Speed 三处 `entry().or_insert(default).<field> = …` 样板。
    fn edit_style(&self, key: StyleKey, edit: impl FnOnce(&mut StateStyle)) {
        let mut s = self.ivars().settings.borrow_mut();
        edit(s.styles.entry(key).or_insert(key.default_style()));
    }

    /// 关闭旧设置窗、丢弃其 pane/控件引用,按当前(可能已变的语言/设置)重新构建并显示。
    fn rebuild_settings(&self) {
        if let Some(w) = self.ivars().settings_window.borrow_mut().take() {
            let _: () = unsafe { msg_send![&w, close] };
        }
        *self.ivars().settings_panes.borrow_mut() = None;
        *self.ivars().settings_sidebar.borrow_mut() = None;
        *self.ivars().settings_content.borrow_mut() = None;
        *self.ivars().settings_scroll.borrow_mut() = None;
        self.ivars().settings_pane_heights.borrow_mut().clear();
        *self.ivars().settings_selected.borrow_mut() = 0;
        self.ivars().state_controls.borrow_mut().clear();
        let w = crate::settings::build(self);
        *self.ivars().settings_window.borrow_mut() = Some(w);
        if let Some(w) = self.ivars().settings_window.borrow().as_ref() {
            crate::settings::show(w);
        }
    }
}

/// AgentStatus 的本地化名称(系统通知 body 用)。与 settings strings 的状态名一致(中/英)。
fn status_name(st: AgentStatus, lang: Lang) -> &'static str {
    use AgentStatus::*;
    match (st, lang) {
        (Working, Lang::Zh) => "运行中",
        (Working, Lang::En) => "Working",
        (NeedsDeci, Lang::Zh) => "待决策",
        (NeedsDeci, Lang::En) => "Pending",
        (Done, Lang::Zh) => "已完成",
        (Done, Lang::En) => "Done",
        (Error, Lang::Zh) => "错误",
        (Error, Lang::En) => "Error",
        (Offline, Lang::Zh) => "异常",
        (Offline, Lang::En) => "Offline",
    }
}

impl AppDelegate {
    /// 状态转入边沿检测:global 与上一轮不同 且 在 `notify_on` 列表 → 发 macOS 系统通知。
    fn maybe_notify(&self, snap: &Snapshot) {
        let st = snap.global;
        let prev = self.ivars().last_global.replace(Some(st));
        if prev != Some(st) && self.ivars().settings.borrow().notify_on.contains(&st) {
            let lang = self.ivars().settings.borrow().lang;
            crate::notify::send("Asig", status_name(st, lang));
        }
    }

    /// 把单个灯效分发到菜单栏灯 + 浮窗(渲染总在主线程)。`render` 与 `preview_tick` 共用,
    /// 避免两处各写一遍 status_item + overlay 的 set_light。
    fn render_anim(&self, anim: LightAnim, layers: u8) {
        let mtm = MainThreadMarker::new().expect("render_anim 须在主线程");
        if let Some(item) = self.ivars().status_item.borrow().as_ref() {
            crate::tray::set_light(item, &anim, mtm);
        }
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_light(view, anim, layers);
        }
    }

    /// 把快照渲染到所有 UI(菜单栏灯 + 浮窗 + popover)。灯效来自用户设置。
    fn render(&self, snap: &Snapshot) {
        // 动画规格(LightAnim)与渐变层数是两条正交轴,分别从 settings 取:light() 不带 layers。
        let (anim, layers) = {
            let s = self.ivars().settings.borrow();
            (s.light(snap), s.layers(snap))
        };
        self.render_anim(anim, layers);
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
            .borrow()
            .poll(std::time::Duration::from_secs(secs as u64))
    }

    /// 设置改动后的【轻量重渲染】路径:存盘 + 立即重应用(圆点大小 + 灯效),不等下一轮 tick。
    ///
    /// 三条落盘路径分工:本函数 = 颜色/动效/速度/时长/大小/轮询(只需重渲染);
    /// 语言/主题/ResetAll 因需整面板重建 / 设 `NSApp.appearance`,走直接 `settings.save()`;
    /// 浮窗位置由 `persist_light_pos()` 每轮 tick 节流写(仅变化时落盘)。
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
        self.render_anim(anim, GRADIENT_LAYERS_DEFAULT);
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
        // origin 没动 → 位置不变 → 跳过昂贵的 screen_id_at(枚举所有屏)。仅在窗口实际移动
        // 后才重算 screen_id 并落盘;99% 的 tick 走这条快路径(浮窗静置时不触屏枚举)。
        if self
            .ivars()
            .settings
            .borrow()
            .light_pos
            .is_some_and(|p| p.x == frame.origin.x && p.y == frame.origin.y)
        {
            return;
        }
        let center = NSPoint::new(
            frame.origin.x + frame.size.width / 2.0,
            frame.origin.y + frame.size.height / 2.0,
        );
        let pos = LightPosition {
            x: frame.origin.x,
            y: frame.origin.y,
            screen_id: crate::overlay::screen_id_at(center),
        };
        // borrow_mut 的 RefMut 在此语句结束 drop,故下行 borrow() 安全(无并存可变借用)。
        self.ivars().settings.borrow_mut().light_pos = Some(pos);
        self.ivars().settings.borrow().save();
    }
}

/// 状态签名:全局态 + done_notif + 各会话(id + status)。相同则视为无变化,跳过渲染。
// 普通 Rust 构造器(非 ObjC 方法):alloc → set_ivars → super init。
impl AppDelegate {
    pub fn new(ivars: AppIvars) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send![Self::class(), alloc] };
        let partial = allocated.set_ivars(ivars);
        unsafe { msg_send![super(partial), init] }
    }
}
