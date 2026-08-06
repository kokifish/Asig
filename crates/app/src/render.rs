//! AppDelegate 的渲染 + 轮询取数 helper(从 app_delegate.rs 外移:define_class! 宏内 ObjC
//! method 只转发调用,主文件聚焦 ObjC 类定义 + action;本文件聚焦把 Snapshot 翻译成灯效 +
//! 浮窗位置记忆 + dev 预览 + 设置改动的轻量重渲染)。

use agent_light_core::{
    GRADIENT_LAYERS_DEFAULT, LightAnim, LightPosition, Settings, Snapshot, StyleKey,
};
use objc2::{DefinedClass, MainThreadMarker};
use objc2_foundation::NSPoint;

use crate::app_delegate::AppDelegate;

impl AppDelegate {
    /// 状态转入边沿检测:global 与上一轮不同 且 在 `notify_on` 列表 → 发 macOS 系统通知。
    pub(crate) fn maybe_notify(&self, snap: &Snapshot) {
        let st = snap.global;
        let prev = self.ivars().last_global.replace(Some(st));
        if prev != Some(st) && self.ivars().settings.borrow().notify_on.contains(&st) {
            let lang = self.ivars().settings.borrow().lang;
            crate::notify::send("Asig", crate::settings::strings::status_name(st, lang));
        }
    }

    /// 把单个灯效分发到菜单栏灯 + 浮窗(渲染总在主线程)。`render` 与 `preview_tick` 共用,
    /// 避免两处各写一遍 status_item + overlay 的 set_light。
    pub(crate) fn render_anim(&self, anim: LightAnim, layers: u8) {
        let mtm = MainThreadMarker::new().expect("render_anim 须在主线程");
        if let Some(item) = self.ivars().status_item.borrow().as_ref() {
            crate::tray::set_light(item, &anim, mtm);
        }
        if let Some(view) = self.ivars().overlay_view.borrow().as_ref() {
            crate::overlay::set_light(view, anim, layers);
        }
    }

    /// 把快照渲染到所有 UI(菜单栏灯 + 浮窗 + popover)。灯效来自用户设置。
    pub(crate) fn render(&self, snap: &Snapshot) {
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
    pub(crate) fn snap(&self) -> Snapshot {
        let secs = Settings::done_notif_clamp(self.ivars().settings.borrow().done_notif_duration_s);
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
    pub(crate) fn settings_changed(&self) {
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
    pub(crate) fn preview_tick(&self) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static IDX: AtomicUsize = AtomicUsize::new(0);
        // (名称, 默认动效)——单一事实源:全部经 StyleKey::default_style().to_light(),与真实
        // 渲染(Settings::light → style_for().to_light())同源;DoneNotif 默认不再此处重写第二遍。
        let states: [(&str, LightAnim); 6] = [
            ("Done", StyleKey::Done.default_style().to_light()),
            ("DoneNotif", StyleKey::DoneNotif.default_style().to_light()),
            ("Working", StyleKey::Working.default_style().to_light()),
            ("NeedsDeci", StyleKey::NeedsDeci.default_style().to_light()),
            ("Error", StyleKey::Error.default_style().to_light()),
            ("Offline", StyleKey::Offline.default_style().to_light()),
        ];
        let (name, anim) = states[IDX.fetch_add(1, Ordering::SeqCst) % states.len()];
        self.render_anim(anim, GRADIENT_LAYERS_DEFAULT);
        println!("[asig-preview] {name}: {anim:?}");
        let mut out = std::io::stdout();
        let _ = std::io::Write::flush(&mut out);
    }

    /// 记住浮窗当前位置(全局 origin + 所在屏 id),供下次启动恢复。tick 每 ~3s 调一次,
    /// 仅在位置变化时写盘 —— 比 windowDidMove 更省事,且抗强杀(3s 内必落盘)。
    pub(crate) fn persist_light_pos(&self) {
        let frame = {
            let win = self.ivars().overlay_window.borrow();
            let Some(w) = win.as_ref() else { return };
            w.frame()
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
