//! 最近 start/done 事件:Monitor 边沿检测产出 → VecDeque buffer → Snapshot → Panel 事件列表。
//!
//! 持久化到 `~/Library/Application Support/Asig/recent_events.json`(照抄 `config.rs` 模式:
//! NotFound→空、IO 错 `log::warn`、JSON 损坏备份 `.bad`,**绝不 panic**)。跨 Asig 重启保留。

use crate::persist;
use crate::source::AgentKind;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

/// 最近事件 buffer 容量。Panel 事件列表无行数上限(高度随内容自适应、超出才滚动),
/// 容量即历史保留深度:轮询繁忙的 agent(多会话连转)每天可产出数百条边沿,10 条的
/// 旧值(2026-08-01 面板尚小时所定)几分钟即翻转一轮,把闲置 agent(openclaw 整日无
/// 交互)的记录整个冲出「历史」,面板长期只剩单一 agent 的会话(2026-10-27 实测)。
pub const MAX_EVENTS: usize = 200;
/// 单条事件 content 展示上限(按 char)。Panel 单行宽度有限,入 buffer 前截断,使
/// buffer/持久化/显示共用同一短串(<60MB、持久化不膨胀、单行不溢出)。
const DISPLAY_MAX_CHARS: usize = 48;

/// 事件种类:Start = 会话进入 Working(用户发了消息/开始跑);Done = 进入 Done(回复完成)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Start,
    Done,
}

/// 一条 Panel 事件列表项。serde 持久化(`recent_events.json`,**最新在前**的数组)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub kind: AgentKind,
    /// 会话可读标识(同 `AgentSession::display_label`)。
    pub label: String,
    pub event_kind: EventKind,
    /// 已规范化的消息文本(折叠空白 + 截断)。
    pub content: String,
    /// 边沿发生时刻(epoch ms)。
    pub at_ms: u64,
}

/// 规范化为展示文本:各类空白(换行/制表符/连续空格)合一为单空格、首尾去白、按 char 截断到
/// `DISPLAY_MAX_CHARS` + "…"。短串原样返回。
pub fn truncate_for_display(s: &str) -> String {
    let folded = fold_ws(s);
    let trimmed = folded.trim();
    let mut chars = trimmed.chars();
    let mut head = String::with_capacity(DISPLAY_MAX_CHARS + 1);
    for _ in 0..DISPLAY_MAX_CHARS {
        match chars.next() {
            Some(c) => head.push(c),
            None => return trimmed.to_string(), // 未超长
        }
    }
    if chars.next().is_none() {
        return trimmed.to_string(); // 恰好等于上限,不加 …
    }
    head.push('…');
    head
}

/// 把所有空白字符折叠成单空格(不去首尾)。
fn fold_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

/// 从 `~/Library/Application Support/Asig/recent_events.json` 读(最新在前的 Vec)。**不 panic**:
/// 无文件 → 空;IO 错 / JSON 损坏 → `log::warn` + 备份 `.bad` + 空。语义见 `persist::load_or_default`。
pub fn load() -> Vec<AgentEvent> {
    let Some(p) = persist::app_support_path("recent_events.json") else {
        return Vec::new();
    };
    persist::load_or_default(&p)
}

/// 从指定路径加载(测试 / 注入用)。
pub fn load_at(path: &Path) -> Vec<AgentEvent> {
    persist::load_or_default(path)
}

/// 写「最新在前」的 Vec 到 `recent_events.json`(原子写)。**不 panic**,失败 `log::warn`。
pub fn save(events: &[AgentEvent]) {
    if let Some(p) = persist::app_support_path("recent_events.json") {
        persist::save_json(&p, events);
    }
}

/// 写到指定路径(测试 / 注入用)。
pub fn save_at(path: &Path, events: &[AgentEvent]) {
    persist::save_json(path, events);
}

/// 超容量裁剪:淘汰「条数最多的 agent 种类的最老一条」,而非朴素丢全局最旧。各类 agent
/// 共用这一个历史列表,繁忙种类(zcode 多会话连转)若按 FIFO 淘汰,会把当日闲置种类
/// (openclaw 无交互)的记录成比例挤光 —— 用户看到的「历史会话」只剩繁忙方。按种类
/// 限额淘汰后,一个种类的记录只在它自己是多数派时才会被裁:少数派历史自然留存,总量
/// 仍受 `MAX_EVENTS` 硬约束。单种类时退化为与 FIFO 等价。
pub(crate) fn trim_balanced(buf: &mut VecDeque<AgentEvent>) {
    while buf.len() > MAX_EVENTS {
        // 一次正序扫描统计各种类:条数 + 最老一条的位置(buf 最新在前,下标越大越老;
        // 同种类重复出现时覆盖更新,扫完留下的即最大下标 = 最老)。
        let mut stats: Vec<(AgentKind, usize, usize)> = Vec::new(); // (kind, 条数, 最老下标)
        for (i, e) in buf.iter().enumerate() {
            match stats.iter_mut().find(|(k, _, _)| *k == e.kind) {
                Some(s) => {
                    s.1 += 1;
                    s.2 = i;
                }
                None => stats.push((e.kind, 1, i)),
            }
        }
        // 淘汰对象 = 条数最多的种类;并列取最老下标更大(积压更久)的一方。
        let victim = stats
            .into_iter()
            .reduce(|a, b| if (b.1, b.2) > (a.1, a.2) { b } else { a })
            .expect("len > MAX_EVENTS ≥ 1 时必有统计行");
        buf.remove(victim.2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate_for_display("hello"), "hello");
        assert_eq!(truncate_for_display(""), "");
    }

    #[test]
    fn truncate_folds_whitespace() {
        assert_eq!(truncate_for_display("a\nb\t c  d"), "a b c d");
        assert_eq!(truncate_for_display("  x\ny  "), "x y");
    }

    #[test]
    fn truncate_long_gets_ellipsis() {
        let s = "一".repeat(60);
        let t = truncate_for_display(&s);
        assert_eq!(t.chars().count(), DISPLAY_MAX_CHARS + 1); // 48 字 + …
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_at_boundary_no_ellipsis() {
        let s = "一".repeat(DISPLAY_MAX_CHARS);
        assert_eq!(truncate_for_display(&s), s);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("asig_events_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recent_events.json");
        let events = vec![
            AgentEvent {
                kind: AgentKind::OpenClaw,
                label: "kotomi".into(),
                event_kind: EventKind::Start,
                content: "hi".into(),
                at_ms: 100,
            },
            AgentEvent {
                kind: AgentKind::Claude,
                label: "proj".into(),
                event_kind: EventKind::Done,
                content: "done it".into(),
                at_ms: 200,
            },
        ];
        save_at(&path, &events);
        let back = load_at(&path);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].label, "kotomi");
        assert_eq!(back[0].event_kind, EventKind::Start);
        assert_eq!(back[1].label, "proj");
        assert_eq!(back[1].event_kind, EventKind::Done);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_is_empty() {
        let path =
            std::env::temp_dir().join(format!("asig_events_nope_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(load_at(&path).is_empty());
    }

    #[test]
    fn load_corrupt_json_is_empty_and_backs_up() {
        let dir = std::env::temp_dir().join(format!("asig_events_corrupt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recent_events.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        assert!(load_at(&path).is_empty());
        assert!(dir.join("recent_events.json.bad").exists(), "应备份为 .bad");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn event_serde_roundtrip() {
        let e = AgentEvent {
            kind: AgentKind::Hermes,
            label: "disp".into(),
            event_kind: EventKind::Done,
            content: "c".into(),
            at_ms: 999,
        };
        let text = serde_json::to_string(&e).unwrap();
        let back: AgentEvent = serde_json::from_str(&text).unwrap();
        assert_eq!(back.kind, AgentKind::Hermes);
        assert_eq!(back.event_kind, EventKind::Done);
        assert_eq!(back.at_ms, 999);
    }

    /// 构造一条事件(buf 语义:调用方按最新在前依次 push)。
    fn ev(kind: AgentKind, at_ms: u64) -> AgentEvent {
        AgentEvent {
            kind,
            label: "l".into(),
            event_kind: EventKind::Start,
            content: "c".into(),
            at_ms,
        }
    }

    #[test]
    fn trim_single_kind_behaves_like_fifo() {
        // 单种类:退化为 FIFO —— 保留最新 MAX_EVENTS 条,顺序(最新在前)不变。
        let mut buf: VecDeque<AgentEvent> = (0..(MAX_EVENTS + 30) as u64)
            .rev() // at_ms 大 = 最新;构造后 front 是最新
            .map(|t| ev(AgentKind::Zcode, t))
            .collect();
        trim_balanced(&mut buf);
        assert_eq!(buf.len(), MAX_EVENTS);
        assert_eq!(buf.front().unwrap().at_ms, (MAX_EVENTS + 29) as u64);
        assert_eq!(buf.back().unwrap().at_ms, 30);
        assert!(
            buf.iter()
                .zip(buf.iter().skip(1))
                .all(|(a, b)| a.at_ms > b.at_ms),
            "应保持最新在前"
        );
    }

    #[test]
    fn trim_keeps_minority_kind_history() {
        // 核心场景:繁忙种类(数百条)+ 闲置种类(少量)交错,超容量后闲置种类的记录
        // 不得被挤光 —— 面板「历史只剩单一家」的缺陷(2026-10-27)由此回归锁死。
        let mut buf: VecDeque<AgentEvent> = VecDeque::new();
        let mut t = 0u64;
        for _ in 0..3 {
            // openclaw 少量事件
            buf.push_front(ev(AgentKind::OpenClaw, t));
            t += 1;
            // zcode 大量事件冲刷
            for _ in 0..90 {
                buf.push_front(ev(AgentKind::Zcode, t));
                t += 1;
            }
        }
        // 273 条(openclaw 3 + zcode 270)> 容量:zcode 被裁,openclaw 一条不丢。
        trim_balanced(&mut buf);
        assert_eq!(buf.len(), MAX_EVENTS);
        let oc = buf.iter().filter(|e| e.kind == AgentKind::OpenClaw).count();
        assert_eq!(oc, 3, "少数派种类的历史应全数保留");
        assert_eq!(
            buf.back().unwrap().kind,
            AgentKind::OpenClaw,
            "最老一条应是 openclaw(zcode 的更老者已被裁)"
        );
    }

    #[test]
    fn trim_balances_majority_kinds() {
        // 两方都多:裁剪后向均衡收敛 —— 多数派被裁到与少数派同量级,不会单方独占。
        let mut buf: VecDeque<AgentEvent> = VecDeque::new();
        for i in 0..(MAX_EVENTS as u64 + 50) {
            let kind = if i % 10 < 9 {
                AgentKind::Zcode
            } else {
                AgentKind::Hermes
            };
            buf.push_front(ev(kind, i));
        }
        trim_balanced(&mut buf);
        assert_eq!(buf.len(), MAX_EVENTS);
        let z = buf.iter().filter(|e| e.kind == AgentKind::Zcode).count();
        let h = buf.iter().filter(|e| e.kind == AgentKind::Hermes).count();
        assert_eq!(z + h, MAX_EVENTS);
        // hermes(少数派)至少保住其存量的一半以上;zcode 承担绝大部分裁剪。
        assert!(h >= 20, "少数派应基本留存,实存 {h}");
        assert!(z > h, "多数派仍占大头,实存 zcode={z} hermes={h}");
    }

    #[test]
    fn trim_under_cap_is_noop() {
        let mut buf: VecDeque<AgentEvent> = (0..10).map(|t| ev(AgentKind::Claude, t)).collect();
        trim_balanced(&mut buf);
        assert_eq!(buf.len(), 10);
    }
}
