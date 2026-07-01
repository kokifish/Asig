//! 用户可配置的设置(灯大小 + 各状态样式)。serde 持久化,UI 无关、可移植。
//!
//! 默认值 = status.rs 里 `AgentStatus::light()` 的硬编码映射(5 个真实状态)
//! 以及 Done-Notification 的内置默认(浅蓝快速呼吸)。一旦写入配置文件,app 层就
//! 改读 `Settings::light(&snap)`,不再用硬编码。

use crate::Snapshot;
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

/// 单个状态的可配置样式:颜色 + 动画 + 周期。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StateStyle {
    pub color: Color,
    pub anim: Anim,
    /// 动画周期(ms)。Steady 时无意义,置 0。
    pub period_ms: u32,
}

impl StateStyle {
    /// 反向:从内核硬编码的 `LightAnim` 构造(用于派生 5 个真实状态的默认样式)。
    fn from_light(la: LightAnim) -> Self {
        match la {
            LightAnim::Steady { color } => Self {
                color,
                anim: Anim::Steady,
                period_ms: 0,
            },
            LightAnim::Pulse { color, period_ms } => Self {
                color,
                anim: Anim::Pulse,
                period_ms,
            },
            LightAnim::Ripple { color, period_ms } => Self {
                color,
                anim: Anim::Ripple,
                period_ms,
            },
        }
    }

    /// 正向:翻译成内核的 `LightAnim`(带周期下限保护,避免过快)。
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
}

fn default_poll_interval_ms() -> u32 {
    3000
}

impl Default for Settings {
    fn default() -> Self {
        let styles = StyleKey::ALL
            .iter()
            .map(|&k| (k, k.default_style()))
            .collect();
        Self {
            dot_size: 25,
            styles,
            light_pos: None,
            poll_interval_ms: default_poll_interval_ms(),
            lang: Lang::default(),
            theme: Theme::default(),
        }
    }
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

    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("Asig").join("settings.json"))
    }

    /// 从 `~/Library/Application Support/Asig/settings.json` 读;读不到/损坏则用默认。
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 写回配置文件。失败静默(只读环境也不该崩)。
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&p, text);
            }
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
                color: Color::Purple
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
            },
        );
        assert!(matches!(
            s.light_for(AgentStatus::Done),
            LightAnim::Steady { color: Color::Red }
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
                period_ms: 1200
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
            },
        );
        assert!(
            matches!(s.light_for(AgentStatus::Working), LightAnim::Pulse { period_ms, .. } if period_ms >= 200)
        );
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
                period_ms: 350
            }
        ));
        assert!(matches!(
            s.light_for(AgentStatus::NeedsDeci),
            LightAnim::Pulse {
                color: Color::Amber,
                period_ms: 1000
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
}
