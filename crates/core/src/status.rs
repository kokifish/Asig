//! 统一状态模型 + 状态机 —— core 与 UI 之间的契约。
//! 一个 AgentStatus 同时决定:灯的颜色 + 灯效(动画)。UI 层只消费 `light()`。

use serde::{Deserialize, Serialize};

/// 监控的统一状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,   // 🟡 在跑
    NeedsDeci, // 🟠 待决策(要权限 / 要输入)
    #[default]
    Done, // 🟢 完成 / 空闲 / 初始默认态
    Error,     // 🔴 报错且无法自动恢复
    Offline,   // 🟣 不可观测 / 卡住 / 进程没了 / 未知
}

impl AgentStatus {
    /// 聚合优先级(高者覆盖低者)。多会话压成一颗全局灯时用。
    pub fn priority(self) -> u8 {
        match self {
            Self::Error => 5,
            Self::NeedsDeci => 4,
            Self::Offline => 3,
            Self::Working => 2,
            Self::Done => 1,
        }
    }

    /// 该状态对应的默认灯效(颜色 + 动画)。UI 层据此驱动 CoreAnimation。
    /// 默认动效见 DEV.md「Color and State Priority」表。
    /// 快闪 / 慢闪 / 呼吸 都是 `Pulse`(只是周期不同),无独立的明灭(Blink)动效。
    pub fn light(self) -> LightAnim {
        match self {
            Self::Done => LightAnim::Ripple {
                color: Color::Green,
                period_ms: 1600,
            }, // 波纹
            Self::Working => LightAnim::Pulse {
                color: Color::Yellow,
                period_ms: 1800,
            }, // 呼吸-慢速
            Self::NeedsDeci => LightAnim::Pulse {
                color: Color::Amber,
                period_ms: 1000,
            }, // 慢闪(中速呼吸)
            Self::Error => LightAnim::Pulse {
                color: Color::Red,
                period_ms: 350,
            }, // 快闪(快速呼吸)
            Self::Offline => LightAnim::Steady {
                color: Color::Purple,
            }, // 常亮
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Green, // Done
    /// Done Notification(刚转入 Done 的 30 秒内)。浅蓝。旧配置的 "dark_green" 兼容映射。
    #[serde(alias = "dark_green")]
    LightBlue,
    Yellow, // Working
    Amber,  // NeedsDeci
    Red,    // Error
    Purple, // Offline
    // —— 个性化扩展色(无默认状态映射,仅 Settings 可选;Tailwind 源,见 overlay.rs)——
    Blue,
    Indigo,
    Teal,
    Cyan,
    Orange,
    Pink,
}

/// 灯效规格(平台无关)。app 层翻译成 CoreAnimation。
/// 共 3 种:Steady 常亮 / Pulse 呼吸(快闪·慢闪·呼吸只是周期不同)/ Ripple 波纹。
#[derive(Debug, Clone, Copy)]
pub enum LightAnim {
    Steady { color: Color },                 // 常亮
    Pulse { color: Color, period_ms: u32 },  // 呼吸:透明度在 0.2~1 间渐变(周期越短越「闪」)
    Ripple { color: Color, period_ms: u32 }, // 波纹:环从中心扩散并淡出
}

/// 状态机:把「本轮观测 raw」叠加到「已锁定 current」。
///
/// 规则:
/// - `Done`(基线)/ `Working` 可自由转移 —— 接受任意新观测;
/// - `NeedsDeci` / `Error` / `Offline` 一旦进入即**锁定**,只有明确的
///   `Working`(恢复)或 `Done`(结束)才解锁 —— 不因超时或抖动清掉,
///   也不会在彼此间互相覆盖(先到先得,避免闪烁)。
pub fn transition(current: AgentStatus, raw: AgentStatus) -> AgentStatus {
    use AgentStatus::*;
    match current {
        Done | Working => raw, // 基线 / 运行中:接受任意新观测
        NeedsDeci | Error | Offline => match raw {
            Working | Done => raw, // 锁定态:仅 Working/Done 可解锁
            _ => current,          // 其余保持(不抖动、不超时清)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_ordering() {
        assert!(AgentStatus::Error.priority() > AgentStatus::NeedsDeci.priority());
        assert!(AgentStatus::NeedsDeci.priority() > AgentStatus::Offline.priority());
        assert!(AgentStatus::Offline.priority() > AgentStatus::Working.priority());
        assert!(AgentStatus::Working.priority() > AgentStatus::Done.priority());
    }

    #[test]
    fn light_mapping_matches_dev_doc() {
        // Done=波纹绿 / Working=慢呼吸黄 / NeedsDeci=慢闪琥珀 / Error=快闪红 / Offline=常亮紫
        // 快闪·慢闪·呼吸 都是 Pulse(周期不同),无独立 Blink 动效。
        assert!(matches!(
            AgentStatus::Done.light(),
            LightAnim::Ripple {
                color: Color::Green,
                ..
            }
        ));
        assert!(matches!(
            AgentStatus::Working.light(),
            LightAnim::Pulse {
                color: Color::Yellow,
                ..
            }
        ));
        assert!(matches!(
            AgentStatus::NeedsDeci.light(),
            LightAnim::Pulse {
                color: Color::Amber,
                ..
            }
        ));
        assert!(matches!(
            AgentStatus::Error.light(),
            LightAnim::Pulse {
                color: Color::Red,
                ..
            }
        ));
        assert!(matches!(
            AgentStatus::Offline.light(),
            LightAnim::Steady {
                color: Color::Purple
            }
        ));
        // 快闪(Error)周期 < 慢闪(NeedsDeci)周期 < 呼吸(Working)
        let err = matches!(AgentStatus::Error.light(), LightAnim::Pulse { period_ms, .. } if period_ms < 600);
        let nd = matches!(AgentStatus::NeedsDeci.light(), LightAnim::Pulse { period_ms: p, .. } if (600..1500).contains(&p));
        assert!(err && nd);
        assert!(
            matches!(AgentStatus::Working.light(), LightAnim::Pulse { period_ms, .. } if period_ms >= 1500)
        );
    }

    #[test]
    fn transition_free_from_baseline() {
        // Done / Working 接受任意新观测
        assert_eq!(
            transition(AgentStatus::Done, AgentStatus::Error),
            AgentStatus::Error
        );
        assert_eq!(
            transition(AgentStatus::Working, AgentStatus::NeedsDeci),
            AgentStatus::NeedsDeci
        );
        assert_eq!(
            transition(AgentStatus::Done, AgentStatus::Working),
            AgentStatus::Working
        );
    }

    #[test]
    fn transition_sticky_unlocks_only_on_working_or_done() {
        // 锁定态:仅 Working/Done 可解锁
        assert_eq!(
            transition(AgentStatus::Error, AgentStatus::Working),
            AgentStatus::Working
        );
        assert_eq!(
            transition(AgentStatus::Offline, AgentStatus::Done),
            AgentStatus::Done
        );
        // 其余原始观测一律保持(不抖动、不互相覆盖、不超时清)
        assert_eq!(
            transition(AgentStatus::Error, AgentStatus::Offline),
            AgentStatus::Error
        );
        assert_eq!(
            transition(AgentStatus::Offline, AgentStatus::NeedsDeci),
            AgentStatus::Offline
        );
        assert_eq!(
            transition(AgentStatus::NeedsDeci, AgentStatus::Error),
            AgentStatus::NeedsDeci
        );
        // 同为锁定态之间也不互相覆盖
        assert_eq!(
            transition(AgentStatus::Error, AgentStatus::NeedsDeci),
            AgentStatus::Error
        );
    }

    #[test]
    fn color_serde_roundtrip_all_12() {
        // 12 色 serde rename snake_case 往返(含新增 6 个性化色)。
        let cases = [
            ("green", Color::Green),
            ("light_blue", Color::LightBlue),
            ("yellow", Color::Yellow),
            ("amber", Color::Amber),
            ("red", Color::Red),
            ("purple", Color::Purple),
            ("blue", Color::Blue),
            ("indigo", Color::Indigo),
            ("teal", Color::Teal),
            ("cyan", Color::Cyan),
            ("orange", Color::Orange),
            ("pink", Color::Pink),
        ];
        for (name, c) in cases {
            let s = serde_json::to_string(&c).unwrap();
            assert_eq!(s, format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<Color>(&s).unwrap(), c);
        }
    }
}
