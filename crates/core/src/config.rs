//! 用户可配置的设置(灯大小 + 各状态样式)。serde 持久化,UI 无关、可移植。
//!
//! 默认值 = status.rs 里 `AgentStatus::light()` 的硬编码映射(5 个真实状态)
//! 以及 Done-Notification 的内置默认(浅蓝快速呼吸)。一旦写入配置文件,app 层就
//! 改读 `Settings::light(&snap)`,不再用硬编码。

use crate::Snapshot;
use crate::source::AgentKind;
use crate::status::{AgentStatus, Color, LightAnim};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 灯效类型(与 `LightAnim` 的变体对应,但去掉了 color/period —— 那俩放 `StateStyle`)。
/// 共 3 种:快闪 / 慢闪 / 呼吸都是 `Pulse`(只是周期不同),故无独立 Blink。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anim {
    Steady,
    /// 兼容旧配置文件里写过的 `blink` —— 旧值一律按呼吸(Pulse)解释。
    #[serde(alias = "blink")]
    Pulse,
    Ripple,
}

/// 渐变层数(信号灯圆点同心圆分层)的合法范围与默认。存的是「slider 值」0..=4:
/// 0=纯色单层(等价历史行为)、1=两层(外层 α=0.5)、2=三层(中 2/3·外 1/3)……
/// 实际层数 L=layers+1,第 k 层(k=0 中心)透明度 α=1−k/L。app 层 slider 以 MIN/MAX 为边界,
/// 渲染层 draw_rect 据此画等距同心环;仅作用于浮窗圆点本体,菜单栏图标不分级。
pub const GRADIENT_LAYERS_MIN: u8 = 0;
pub const GRADIENT_LAYERS_MAX: u8 = 4;
pub const GRADIENT_LAYERS_DEFAULT: u8 = 1;

/// 单个状态的可配置样式:颜色 + 动画 + 周期 + 渐变层数。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StateStyle {
    pub color: Color,
    pub anim: Anim,
    /// 动画周期(ms)。Steady 时无意义,置 0。
    pub period_ms: u32,
    /// 渐变层数(slider 值 0..=4,见 `status::GRADIENT_LAYERS_*`)。浮窗圆点据此画 layers+1 同心环;
    /// 旧配置缺此字段 → 默认 1(两层渐变)。
    #[serde(default = "default_gradient_layers")]
    pub gradient_layers: u8,
}

impl StateStyle {
    /// 反向:从内核硬编码的 `LightAnim` 构造(用于派生 5 个真实状态的默认样式)。
    fn from_light(la: LightAnim) -> Self {
        // LightAnim 不带渐变层数(那是正交的圆点绘制规格),派生默认样式时回填默认层数。
        let (color, anim, period_ms) = match la {
            LightAnim::Steady { color } => (color, Anim::Steady, 0),
            LightAnim::Pulse { color, period_ms } => (color, Anim::Pulse, period_ms),
            LightAnim::Ripple { color, period_ms } => (color, Anim::Ripple, period_ms),
        };
        Self {
            color,
            anim,
            period_ms,
            gradient_layers: GRADIENT_LAYERS_DEFAULT,
        }
    }

    /// 正向:翻译成内核的 `LightAnim`(带周期下限保护,避免过快)。不含渐变层数——
    /// 那是正交的圆点绘制规格,由 `layers()` 单独取,经 `set_light` 参数传入浮窗。
    fn to_light(self) -> LightAnim {
        match self.anim {
            Anim::Steady => LightAnim::Steady { color: self.color },
            Anim::Pulse => LightAnim::Pulse {
                color: self.color,
                period_ms: self.period_ms.max(200),
            },
            Anim::Ripple => LightAnim::Ripple {
                color: self.color,
                period_ms: self.period_ms.max(400),
            },
        }
    }

    /// 渐变层数(slider 值,clamp 到合法范围)。浮窗 drawRect 据此画 layers+1 同心环。
    pub fn layers(self) -> u8 {
        self.gradient_layers
            .clamp(GRADIENT_LAYERS_MIN, GRADIENT_LAYERS_MAX)
    }
}

/// 界面语言。默认中文。serde 持久化,切换后整个 Settings Panel 重绘。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Zh,
    En,
}

/// 界面外观主题(跟随系统 / 深色 / 浅色)。默认跟随系统。serde 持久化;
/// 切换后 app 层立即设 `NSApp.appearance` 并触发重绘。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    FollowSystem,
    Dark,
    Light,
}

/// 可配置灯效的键:5 个真实 `AgentStatus` + Done-Notification(派生态,非真实状态)。
/// 用它统一做 `Settings` 的键 + Settings Panel 的行,避免给 Done-Notification 特判。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleKey {
    Done,
    Working,
    NeedsDeci,
    Error,
    Offline,
    /// Done-Notification:别的态刚转入 Done 的窗口期内的覆盖灯效。
    DoneNotif,
}

impl StyleKey {
    /// Settings Panel 里的固定顺序(与下拉 tag 编码一致;app_delegate 解码也用它)。
    pub const ALL: [Self; 6] = [
        Self::Done,
        Self::Working,
        Self::NeedsDeci,
        Self::Error,
        Self::Offline,
        Self::DoneNotif,
    ];

    /// 对应的真实状态;Done-Notification 返回 None。
    pub fn status(self) -> Option<AgentStatus> {
        match self {
            Self::Done => Some(AgentStatus::Done),
            Self::Working => Some(AgentStatus::Working),
            Self::NeedsDeci => Some(AgentStatus::NeedsDeci),
            Self::Error => Some(AgentStatus::Error),
            Self::Offline => Some(AgentStatus::Offline),
            Self::DoneNotif => None,
        }
    }

    /// 内置默认样式。5 个真实状态派生自 `AgentStatus::light()`(单一事实源);
    /// Done-Notification 默认 = 浅蓝快速呼吸(内置于 `StyleKey::default_style`)。
    pub fn default_style(self) -> StateStyle {
        match self {
            Self::DoneNotif => StateStyle {
                color: Color::LightBlue,
                anim: Anim::Pulse,
                period_ms: 450,
                gradient_layers: GRADIENT_LAYERS_DEFAULT,
            },
            other => StateStyle::from_light(other.status().unwrap().light()),
        }
    }
}

impl From<AgentStatus> for StyleKey {
    fn from(s: AgentStatus) -> Self {
        match s {
            AgentStatus::Done => Self::Done,
            AgentStatus::Working => Self::Working,
            AgentStatus::NeedsDeci => Self::NeedsDeci,
            AgentStatus::Error => Self::Error,
            AgentStatus::Offline => Self::Offline,
        }
    }
}

/// Signal Light 浮窗位置(全局屏幕坐标 + 所在屏幕 ID)。用于跨启动记忆。
/// `screen_id` 是 CGDirectDisplayID;恢复时按它定位上次所在的屏幕,若该屏已断开
/// 则回退到主屏左上角默认位。`0` 表示未知屏幕。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LightPosition {
    pub x: f64,
    pub y: f64,
    pub screen_id: u32,
}

/// Done-Notification 持续时间(秒)的合法范围与默认。app 层 slider 与内核读取共用同一组
/// 常量:slider 以 MIN/MAX 为边界,内核 poll 据 DEFAULT 兜底、读取时 clamp 到此范围。
pub const DONE_NOTIF_DURATION_MIN_S: u32 = 5;
pub const DONE_NOTIF_DURATION_MAX_S: u32 = 60;
pub const DONE_NOTIF_DURATION_DEFAULT_S: u32 = 30;

/// 浮窗圆点直径(px)的合法范围与默认。app 层 slider 以 MIN/MAX 为边界、changeSize 读取时
/// clamp 到此范围,默认值兜底 —— slider 与 clamp 共用同一组常量,避免边界两处分写而漂移。
pub const DOT_SIZE_MIN_PX: u32 = 5;
pub const DOT_SIZE_MAX_PX: u32 = 50;
pub const DOT_SIZE_DEFAULT_PX: u32 = 25;

/// 全部设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 浮窗圆点直径(px)。app 层据此重绘。
    pub dot_size: u32,
    /// 各「可配置灯效键」的样式。缺某键时回退到 `StyleKey::default_style()`。
    pub styles: HashMap<StyleKey, StateStyle>,
    /// Signal Light 浮窗位置(跨启动记忆)。缺省(None)→ 主屏左上角默认位。
    #[serde(default)]
    pub light_pos: Option<LightPosition>,
    /// 轮询间隔(ms)。DEV.md 默认 3s。app 层据此重排 tick 定时器。
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u32,
    /// 界面语言。默认中文。
    #[serde(default)]
    pub lang: Lang,
    /// 界面外观主题。默认跟随系统。
    #[serde(default)]
    pub theme: Theme,
    /// Done-Notification 持续时间(秒)。别的态转入 Done 后,该秒数内显示 DoneNotif 灯效。
    /// 默认 30(`DONE_NOTIF_DURATION_DEFAULT_S`),合法范围 5–60。serde 持久化。
    #[serde(default = "default_done_notif_duration_s")]
    pub done_notif_duration_s: u32,
    /// 启用的 agent 列表(General 多选块,选中=监控该 Agent)。默认全部启用 ——
    /// 旧配置无此字段 → 全开,保持现有行为不变。
    #[serde(default = "default_enabled_agents")]
    pub enabled_agents: Vec<AgentKind>,
}

fn default_poll_interval_ms() -> u32 {
    3000
}

fn default_done_notif_duration_s() -> u32 {
    DONE_NOTIF_DURATION_DEFAULT_S
}

fn default_enabled_agents() -> Vec<AgentKind> {
    AgentKind::IMPLEMENTED.to_vec()
}

fn default_gradient_layers() -> u8 {
    GRADIENT_LAYERS_DEFAULT
}

impl Default for Settings {
    fn default() -> Self {
        let styles = StyleKey::ALL
            .iter()
            .map(|&k| (k, k.default_style()))
            .collect();
        Self {
            dot_size: DOT_SIZE_DEFAULT_PX,
            styles,
            light_pos: None,
            poll_interval_ms: default_poll_interval_ms(),
            lang: Lang::default(),
            theme: Theme::default(),
            done_notif_duration_s: default_done_notif_duration_s(),
            enabled_agents: default_enabled_agents(),
        }
    }
}

/// 设置加载失败的原因(诊断用)。`Settings::load` 据此提示用户后回退默认,绝不 panic。
enum LoadError {
    Read(std::io::Error),
    Parse(serde_json::Error),
}

impl Settings {
    /// 某个键对应的样式。配置缺失时回退到内置默认。
    pub fn style_for(&self, key: StyleKey) -> StateStyle {
        self.styles
            .get(&key)
            .copied()
            .unwrap_or_else(|| key.default_style())
    }

    /// 某个真实状态对应的灯效。
    pub fn light_for(&self, s: AgentStatus) -> LightAnim {
        self.style_for(StyleKey::from(s)).to_light()
    }

    /// 一次快照应渲染的灯效:Done-Notification(可配)优先于 global 默认。
    pub fn light(&self, snap: &Snapshot) -> LightAnim {
        if snap.done_notif {
            self.style_for(StyleKey::DoneNotif).to_light()
        } else {
            self.light_for(snap.global)
        }
    }

    /// 某个真实状态对应的渐变层数。
    pub fn layers_for(&self, s: AgentStatus) -> u8 {
        self.style_for(StyleKey::from(s)).layers()
    }

    /// 一次快照应渲染的渐变层数(与 `light()` 同优先级:DoneNotif 优先于 global)。
    pub fn layers(&self, snap: &Snapshot) -> u8 {
        if snap.done_notif {
            self.style_for(StyleKey::DoneNotif).layers()
        } else {
            self.layers_for(snap.global)
        }
    }

    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("Asig").join("settings.json"))
    }

    /// 从指定路径加载,区分"读失败 / 解析失败"(无文件由 `load` 按 NotFound 判定)。
    fn load_result(path: &std::path::Path) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(LoadError::Read)?;
        serde_json::from_str(&text).map_err(LoadError::Parse)
    }

    /// 从 `~/Library/Application Support/Asig/settings.json` 读。**不 panic**:
    /// 无文件 → 静默默认(首次运行);权限/磁盘 IO 错 → eprintln 提示 + 默认;
    /// JSON 损坏 → 把坏文件备份成 `settings.json.bad` + 提示 + 默认(避免下次还解析失败)。
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match Self::load_result(&path) {
            Ok(s) => s,
            Err(LoadError::Read(e)) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(LoadError::Read(e)) => {
                eprintln!("Asig: 读取设置失败({e}),使用默认值: {}", path.display());
                Self::default()
            }
            Err(LoadError::Parse(e)) => {
                eprintln!("Asig: settings.json 解析失败({e}),已备份为 .bad 并使用默认值");
                let _ = std::fs::rename(&path, format!("{}.bad", path.display()));
                Self::default()
            }
        }
    }

    /// 写回配置文件。**不 panic**(只读环境也不该崩),但失败 eprintln 提示 —— 磁盘满/权限错时
    /// 改动"看似生效但不落盘"会害调试,需可见。
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Asig: 创建设置目录失败({e})");
                return;
            }
        }
        let text = match serde_json::to_string_pretty(self) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Asig: 序列化设置失败({e})");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("Asig: 写入设置失败({e}): {}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_builtin_light() {
        let s = Settings::default();
        assert!(matches!(
            s.light_for(AgentStatus::Done),
            LightAnim::Ripple {
                color: Color::Green,
                ..
            }
        ));
        assert!(matches!(
            s.light_for(AgentStatus::Working),
            LightAnim::Pulse {
                color: Color::Yellow,
                ..
            }
        ));
        assert!(matches!(
            s.light_for(AgentStatus::Offline),
            LightAnim::Steady {
                color: Color::Purple,
                ..
            }
        ));
    }

    #[test]
    fn done_notif_default_is_light_blue_fast_pulse() {
        // Done-Notification 默认 = 浅蓝、快速呼吸(与 DEV.md 一致)
        let st = StyleKey::DoneNotif.default_style();
        assert_eq!(st.color, Color::LightBlue);
        assert_eq!(st.anim, Anim::Pulse);
        assert_eq!(st.period_ms, 450);
    }

    #[test]
    fn override_changes_style() {
        let mut s = Settings::default();
        // 把 Done 改成红色常亮
        s.styles.insert(
            StyleKey::Done,
            StateStyle {
                color: Color::Red,
                anim: Anim::Steady,
                period_ms: 0,
                gradient_layers: GRADIENT_LAYERS_DEFAULT,
            },
        );
        assert!(matches!(
            s.light_for(AgentStatus::Done),
            LightAnim::Steady {
                color: Color::Red,
                ..
            }
        ));
    }

    #[test]
    fn override_done_notif_style() {
        // Done-Notification 也能改:这里改成绿色波纹
        let mut s = Settings::default();
        s.styles.insert(
            StyleKey::DoneNotif,
            StateStyle {
                color: Color::Green,
                anim: Anim::Ripple,
                period_ms: 1200,
                gradient_layers: GRADIENT_LAYERS_DEFAULT,
            },
        );
        let snap = Snapshot {
            sessions: vec![],
            global: AgentStatus::Done,
            done_notif: true,
        };
        assert!(matches!(
            s.light(&snap),
            LightAnim::Ripple {
                color: Color::Green,
                period_ms: 1200,
                ..
            }
        ));
    }

    #[test]
    fn missing_state_falls_back() {
        let mut s = Settings::default();
        s.styles.remove(&StyleKey::Error);
        assert!(matches!(
            s.light_for(AgentStatus::Error),
            LightAnim::Pulse {
                color: Color::Red,
                ..
            }
        ));
    }

    #[test]
    fn serialize_roundtrip() {
        let s = Settings::default();
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.dot_size, 25);
        assert_eq!(back.poll_interval_ms, 3000);
        assert_eq!(back.theme, Theme::FollowSystem); // 默认主题序列化往返
        assert_eq!(back.done_notif_duration_s, 30); // 默认持续时间往返
        assert!(back.styles.contains_key(&StyleKey::Done));
        assert!(back.styles.contains_key(&StyleKey::DoneNotif)); // 新增键也序列化
    }

    #[test]
    fn backward_compat_old_keys_deserialize() {
        // 旧配置文件只有 5 个状态键(无 done_notif),应能正常加载并回退默认。
        let old = r#"{"dot_size":20,"styles":{"done":{"color":"green","anim":"ripple","period_ms":1600},"working":{"color":"yellow","anim":"pulse","period_ms":1800}}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.dot_size, 20);
        assert_eq!(s.poll_interval_ms, 3000); // 旧配置无该字段 → 默认 3s
        assert_eq!(s.theme, Theme::FollowSystem); // 旧配置无 theme → 默认跟随系统
        assert_eq!(s.done_notif_duration_s, 30); // 旧配置无该字段 → 默认 30s
        assert!(matches!(
            s.light_for(AgentStatus::Done),
            LightAnim::Ripple { .. }
        ));
        // done_notif 缺失 → 默认浅蓝呼吸
        assert!(matches!(
            s.style_for(StyleKey::DoneNotif),
            StateStyle {
                color: Color::LightBlue,
                anim: Anim::Pulse,
                ..
            }
        ));
        // 旧 styles 子对象缺 gradient_layers → serde 默认 1(两层渐变)
        assert_eq!(s.style_for(StyleKey::Done).gradient_layers, 1);
    }

    #[test]
    fn period_clamped_to_minimum() {
        let mut s = Settings::default();
        s.styles.insert(
            StyleKey::Working,
            StateStyle {
                color: Color::Yellow,
                anim: Anim::Pulse,
                period_ms: 1,
                gradient_layers: GRADIENT_LAYERS_DEFAULT,
            },
        );
        assert!(
            matches!(s.light_for(AgentStatus::Working), LightAnim::Pulse { period_ms, .. } if period_ms >= 200)
        );
    }

    #[test]
    fn gradient_layers_clamped_and_default() {
        // 默认 = 1(两层渐变)
        assert_eq!(Settings::default().style_for(StyleKey::Done).layers(), 1);
        // 越界值(手改配置)经 StateStyle::layers() clamp 回 [0, 4]
        let mut s = Settings::default();
        s.styles.insert(
            StyleKey::Done,
            StateStyle {
                color: Color::Green,
                anim: Anim::Ripple,
                period_ms: 1600,
                gradient_layers: 99,
            },
        );
        assert_eq!(s.style_for(StyleKey::Done).layers(), GRADIENT_LAYERS_MAX);
    }

    #[test]
    fn old_blink_migrates_to_pulse() {
        // 旧配置文件里 Error/NeedsDeci 写的是 "blink";迁移后一律按 Pulse 解释,
        // 周期保留(Error 仍快、NeedsDeci 仍中速)。
        let old = r#"{"dot_size":16,"styles":{
            "error":{"color":"red","anim":"blink","period_ms":350},
            "needs_deci":{"color":"amber","anim":"blink","period_ms":1000}}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(matches!(
            s.light_for(AgentStatus::Error),
            LightAnim::Pulse {
                color: Color::Red,
                period_ms: 350,
                ..
            }
        ));
        assert!(matches!(
            s.light_for(AgentStatus::NeedsDeci),
            LightAnim::Pulse {
                color: Color::Amber,
                period_ms: 1000,
                ..
            }
        ));
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn light_position_optional_and_default() {
        // 无 light_pos 的旧配置 → None(启动用默认左上角)。
        let old = r#"{"dot_size":16,"styles":{}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.light_pos, None);
        // 默认也是 None。
        assert_eq!(Settings::default().light_pos, None);
        // 带 light_pos 能往返。
        let mut s = Settings::default();
        s.light_pos = Some(LightPosition {
            x: 100.0,
            y: 200.0,
            screen_id: 7,
        });
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.light_pos,
            Some(LightPosition {
                x: 100.0,
                y: 200.0,
                screen_id: 7
            })
        );
    }

    #[test]
    fn enabled_agents_default_and_roundtrip() {
        let s = Settings::default();
        assert_eq!(
            s.enabled_agents,
            vec![AgentKind::Claude, AgentKind::CodeBuddy, AgentKind::OpenClaw]
        );
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.enabled_agents, s.enabled_agents);
    }

    #[test]
    fn enabled_agents_backward_compat_absent() {
        // 旧配置无 enabled_agents → 默认全部(现有行为不变)
        let old = r#"{"dot_size":16,"styles":{}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(
            s.enabled_agents,
            vec![AgentKind::Claude, AgentKind::CodeBuddy, AgentKind::OpenClaw]
        );
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn enabled_agents_single_persists() {
        let mut s = Settings::default();
        s.enabled_agents = vec![AgentKind::OpenClaw];
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.enabled_agents, vec![AgentKind::OpenClaw]);
    }

    #[test]
    fn load_result_rejects_corrupt_json() {
        // 损坏的 settings.json 应被 load_result 判为 Parse 失败(load 据此备份 .bad + 回退默认)。
        let path = std::env::temp_dir().join(format!("asig_corrupt_{}.json", std::process::id()));
        std::fs::write(&path, "{ 这是损坏的 json").unwrap();
        assert!(matches!(
            Settings::load_result(&path),
            Err(LoadError::Parse(_))
        ));
        let _ = std::fs::remove_file(&path);
    }
}
