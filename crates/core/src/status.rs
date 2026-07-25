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
                period_ms: 3333, // ≈0.3Hz(1000/0.3);Done 波纹默认速度
            }, // 波纹
            Self::Working => LightAnim::Pulse {
                color: Color::Yellow,
                period_ms: 1800,
            }, // 呼吸-慢速
            Self::NeedsDeci => LightAnim::Ripple {
                color: Color::Amber,
                period_ms: 2500, // ≈0.4Hz,比 Done(3333)稍快
            }, // 波纹(比 Done 稍快)
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

impl Color {
    /// Tailwind 500(浅档)/400(深档)RGB 双档,平台无关 f64。app 据外观选档 + 包 NSColor。
    /// 单一事实源:core 加 Color 变体时此 match 编译强制穷尽(避免 app 漏映射静默漏色)。
    pub fn rgb_pair(self) -> [(f64, f64, f64); 2] {
        match self {
            Self::Green => [(0.133, 0.773, 0.369), (0.290, 0.871, 0.502)], // 22C55E / 4ADE80
            Self::LightBlue => [(0.055, 0.647, 0.914), (0.220, 0.741, 0.973)], // 0EA5E9 / 38BDF8
            Self::Yellow => [(0.918, 0.702, 0.031), (0.980, 0.800, 0.082)], // EAB308 / FACC15
            Self::Amber => [(0.961, 0.620, 0.043), (0.984, 0.749, 0.141)], // F59E0B / FBBF24
            Self::Red => [(0.937, 0.267, 0.267), (0.973, 0.443, 0.443)],   // EF4444 / F87171
            Self::Purple => [(0.659, 0.333, 0.969), (0.753, 0.518, 0.988)], // A855F7 / C084FC
            Self::Blue => [(0.231, 0.510, 0.965), (0.376, 0.647, 0.980)],  // 3B82F6 / 60A5FA
            Self::Indigo => [(0.388, 0.400, 0.945), (0.506, 0.549, 0.973)], // 6366F1 / 818CF8
            Self::Teal => [(0.078, 0.722, 0.651), (0.176, 0.831, 0.749)],  // 14B8A6 / 2DD4BF
            Self::Cyan => [(0.024, 0.714, 0.831), (0.133, 0.827, 0.933)],  // 06B6D4 / 22D3EE
            Self::Orange => [(0.976, 0.451, 0.086), (0.984, 0.573, 0.235)], // F97316 / FB923C
            Self::Pink => [(0.925, 0.282, 0.600), (0.957, 0.447, 0.714)],  // EC4899 / F472B6
        }
    }
}

/// 灯效规格(平台无关)。app 层翻译成 CoreAnimation。
/// 共 3 种:Steady 常亮 / Pulse 呼吸(快闪·慢闪·呼吸只是周期不同)/ Ripple 波纹。
/// 注意:渐变层数**不**在此枚举里——它是与动画正交的"圆点绘制规格",只被浮窗 `drawRect`
/// 消费,故独立为 `set_light` 的参数(见 `StateStyle::layers`),不随 `light()` 流经菜单栏
/// 图标 / 波纹环 / 色块等不分级消费者。
#[derive(Debug, Clone, Copy)]
pub enum LightAnim {
    Steady { color: Color },                 // 常亮
    Pulse { color: Color, period_ms: u32 },  // 呼吸:透明度在 0.2~1 间渐变(周期越短越「闪」)
    Ripple { color: Color, period_ms: u32 }, // 波纹:环从中心扩散并淡出
}

impl LightAnim {
    /// 该灯效的颜色(三种变体都带 color)。菜单栏图标 / 浮窗 / 设置色块共用,避免各处重写 match。
    pub fn color(self) -> Color {
        match self {
            LightAnim::Steady { color } => color,
            LightAnim::Pulse { color, .. } => color,
            LightAnim::Ripple { color, .. } => color,
        }
    }
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
        // Done=波纹绿 / Working=慢呼吸黄 / NeedsDeci=波纹琥珀(比 Done 稍快)/ Error=快闪红 / Offline=常亮紫
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
            LightAnim::Ripple {
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
                color: Color::Purple,
                ..
            }
        ));
        // 快闪(Error)Pulse 周期最短;呼吸(Working)Pulse 周期 ≥1500。
        assert!(
            matches!(AgentStatus::Error.light(), LightAnim::Pulse { period_ms, .. } if period_ms < 600)
        );
        assert!(
            matches!(AgentStatus::Working.light(), LightAnim::Pulse { period_ms, .. } if period_ms >= 1500)
        );
        // NeedsDeci 波纹(2500)比 Done 波纹(3333)稍快。
        let nd = matches!(AgentStatus::NeedsDeci.light(), LightAnim::Ripple { period_ms, .. } if period_ms < 3333);
        let done = matches!(AgentStatus::Done.light(), LightAnim::Ripple { period_ms, .. } if period_ms == 3333);
        assert!(nd && done);
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
