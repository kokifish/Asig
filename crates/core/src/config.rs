//! 用户可配置的设置(灯大小 + 各状态样式)。serde 持久化,UI 无关、可移植。
//!
//! 默认值 = status.rs 里 `AgentStatus::light()` 的硬编码映射;一旦写入配置文件,
//! app 层就改读 `Settings::light(&snap)`,不再用硬编码。

use crate::status::{AgentStatus, Color, LightAnim};
use crate::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 灯效类型(与 `LightAnim` 的变体对应,但去掉了 color/period —— 那俩放 `StateStyle`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anim {
    Steady,
    Pulse,
    Blink,
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

/// 全部设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 浮窗圆点直径(px)。app 层据此重绘。
    pub dot_size: u32,
    /// 各状态的样式。缺某状态时回退到 `AgentStatus::light()`。
    pub styles: HashMap<AgentStatus, StateStyle>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut styles = HashMap::new();
        // 与 status.rs::AgentStatus::light() 一致(首个事实源仍是那里)。
        styles.insert(AgentStatus::Done, st(Color::Green, Anim::Ripple, 1600));
        styles.insert(AgentStatus::Working, st(Color::Yellow, Anim::Pulse, 1800));
        styles.insert(AgentStatus::NeedsDeci, st(Color::Amber, Anim::Blink, 1000));
        styles.insert(AgentStatus::Error, st(Color::Red, Anim::Blink, 350));
        styles.insert(AgentStatus::Offline, st(Color::Purple, Anim::Steady, 0));
        Self { dot_size: 16, styles }
    }
}

fn st(color: Color, anim: Anim, period_ms: u32) -> StateStyle {
    StateStyle { color, anim, period_ms }
}

impl Settings {
    /// 某个状态对应的灯效。配置缺失时回退到内置默认。
    pub fn light_for(&self, s: AgentStatus) -> LightAnim {
        match self.styles.get(&s) {
            Some(st) => match st.anim {
                Anim::Steady => LightAnim::Steady { color: st.color },
                Anim::Pulse => LightAnim::Pulse { color: st.color, period_ms: st.period_ms.max(200) },
                Anim::Blink => LightAnim::Blink { color: st.color, period_ms: st.period_ms.max(100) },
                Anim::Ripple => LightAnim::Ripple { color: st.color, period_ms: st.period_ms.max(400) },
            },
            None => s.light(),
        }
    }

    /// 一次快照应渲染的灯效:Done Notification(深绿快速呼吸)优先于 global 默认。
    pub fn light(&self, snap: &Snapshot) -> LightAnim {
        if snap.done_notif {
            LightAnim::Pulse { color: Color::DarkGreen, period_ms: 450 }
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
        assert!(matches!(s.light_for(AgentStatus::Done), LightAnim::Ripple { color: Color::Green, .. }));
        assert!(matches!(s.light_for(AgentStatus::Working), LightAnim::Pulse { color: Color::Yellow, .. }));
        assert!(matches!(s.light_for(AgentStatus::Offline), LightAnim::Steady { color: Color::Purple }));
    }

    #[test]
    fn override_changes_style() {
        let mut s = Settings::default();
        // 把 Done 改成红色常亮
        s.styles.insert(AgentStatus::Done, st(Color::Red, Anim::Steady, 0));
        assert!(matches!(s.light_for(AgentStatus::Done), LightAnim::Steady { color: Color::Red }));
    }

    #[test]
    fn missing_state_falls_back() {
        let mut s = Settings::default();
        s.styles.remove(&AgentStatus::Error);
        assert!(matches!(s.light_for(AgentStatus::Error), LightAnim::Blink { color: Color::Red, .. }));
    }

    #[test]
    fn serialize_roundtrip() {
        let s = Settings::default();
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.dot_size, 16);
        assert!(back.styles.contains_key(&AgentStatus::Done));
    }

    #[test]
    fn period_clamped_to_minimum() {
        let mut s = Settings::default();
        s.styles.insert(AgentStatus::Working, st(Color::Yellow, Anim::Pulse, 1)); // 太小
        assert!(matches!(s.light_for(AgentStatus::Working), LightAnim::Pulse { period_ms, .. } if period_ms >= 200));
    }
}
