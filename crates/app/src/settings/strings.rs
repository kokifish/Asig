//! 设置窗口界面文案(按 `Settings.lang` 本地化,默认中文,可切全英文)。

use agent_light_core::Lang;

/// 当前语言的全部界面文案。
pub(crate) struct Strings {
    pub(crate) general: &'static str,
    pub(crate) about: &'static str,
    pub(crate) light_size: &'static str,
    pub(crate) click_through: &'static str,
    pub(crate) poll_interval: &'static str,
    pub(crate) launch_login: &'static str,
    pub(crate) language: &'static str,
    pub(crate) theme: &'static str,
    pub(crate) theme_opts: [&'static str; 3],
    pub(crate) reset: &'static str,
    pub(crate) reset_all: &'static str,
    pub(crate) color: &'static str,
    pub(crate) animation: &'static str,
    pub(crate) speed: &'static str,
    pub(crate) gradient: &'static str,
    pub(crate) duration: &'static str,
    pub(crate) version: &'static str,
    pub(crate) state: [&'static str; 6], // 与 STATE_KEYS 同序
    pub(crate) anim: [&'static str; 3],
    pub(crate) poll_opts: [&'static str; 6],
    pub(crate) agent_monitor: &'static str,
    pub(crate) agent_opts: [&'static str; 3],
    pub(crate) notify: &'static str,
    pub(crate) notify_opts: [&'static str; 5],
    pub(crate) hide_in_fullscreen: &'static str,
    pub(crate) reset_confirm_title: &'static str,
    pub(crate) reset_confirm_msg: &'static str,
    pub(crate) reset_yes: &'static str,
    pub(crate) reset_no: &'static str,
}

/// General「Reset 全部」确认对话框的文案(按当前语言):(title, msg, yes, no)。
pub fn reset_confirm_texts(l: Lang) -> (&'static str, &'static str, &'static str, &'static str) {
    let s = strings_for(l);
    (
        s.reset_confirm_title,
        s.reset_confirm_msg,
        s.reset_yes,
        s.reset_no,
    )
}

pub(crate) fn strings_for(l: Lang) -> Strings {
    match l {
        Lang::Zh => Strings {
            general: "常规设置",
            about: "关于",
            light_size: "浮窗灯大小",
            click_through: "点击穿透(取消可拖动)",
            poll_interval: "Agent状态轮询间隔",
            launch_login: "开机自启动(待实现)",
            language: "语言",
            theme: "主题",
            theme_opts: ["跟随系统", "深色", "浅色"],
            reset: "重置",
            reset_all: "重置所有",
            color: "颜色",
            animation: "效果",
            speed: "速度",
            gradient: "渐变层数",
            duration: "持续时间",
            version: "版本 ",
            state: ["完成通知", "已完成", "运行中", "待决策", "错误", "异常"],
            anim: ["常亮", "呼吸", "波纹"],
            poll_opts: ["1 秒", "2 秒", "3 秒", "5 秒", "10 秒", "15 秒"],
            agent_monitor: "监控的 Agent",
            agent_opts: ["Claude Code", "OpenClaw", "Hermes"],
            notify: "状态通知",
            notify_opts: ["已完成", "运行中", "待决策", "错误", "异常"],
            hide_in_fullscreen: "全屏自动隐藏",
            reset_confirm_title: "重置全部设置",
            reset_confirm_msg: "将所有自定义(语言 + 各状态灯效)恢复为默认值。确认?",
            reset_yes: "重置",
            reset_no: "取消",
        },
        Lang::En => Strings {
            general: "General Settings",
            about: "About",
            light_size: "Light size",
            click_through: "Click-through (off = draggable)",
            poll_interval: "Agent poll interval",
            launch_login: "Launch at login (TBD)",
            language: "Language",
            theme: "Theme",
            theme_opts: ["Auto", "Dark", "Light"],
            reset: "Reset",
            reset_all: "Reset All",
            color: "Color",
            animation: "Animation",
            speed: "Speed",
            gradient: "Gradient layers",
            duration: "Duration",
            version: "Version ",
            state: ["Notify", "Done", "Working", "Pending", "Error", "Offline"],
            anim: ["Steady", "Pulse", "Ripple"],
            poll_opts: ["1 s", "2 s", "3 s", "5 s", "10 s", "15 s"],
            agent_monitor: "Agent to monitor",
            agent_opts: ["Claude Code", "OpenClaw", "Hermes"],
            notify: "Status notifications",
            notify_opts: ["Done", "Working", "Pending", "Error", "Offline"],
            hide_in_fullscreen: "Hide in fullscreen",
            reset_confirm_title: "Reset all settings",
            reset_confirm_msg: "Restore all custom settings (language + per-state styles) to defaults?",
            reset_yes: "Reset",
            reset_no: "Cancel",
        },
    }
}
