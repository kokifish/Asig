//! 最近 start/done 事件:Monitor 边沿检测产出 → VecDeque buffer → Snapshot → Panel 事件列表。
//!
//! 持久化到 `~/Library/Application Support/Asig/recent_events.json`(照抄 `config.rs` 模式:
//! NotFound→空、IO 错 `log::warn`、JSON 损坏备份 `.bad`,**绝不 panic**)。跨 Asig 重启保留。

use crate::source::AgentKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 最近事件 buffer 容量(超出按时间淘汰最旧)。
pub const MAX_EVENTS: usize = 10;
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

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Asig").join("recent_events.json"))
}

/// 从 `~/Library/Application Support/Asig/recent_events.json` 读(最新在前的 Vec)。**不 panic**:
/// 无文件 → 空(首次运行);IO 错 / JSON 损坏 → `log::warn` + 备份 `.bad` + 空。
pub fn load() -> Vec<AgentEvent> {
    path().map(|p| load_at(&p)).unwrap_or_default()
}

/// 从指定路径加载(测试 / 注入用)。
pub fn load_at(path: &Path) -> Vec<AgentEvent> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(), // 无文件 / 读失败 → 空(首次运行或权限问题,静默)
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            // 文件存在但解析失败:多半是损坏。备份 .bad(照抄 config.rs)避免下次还失败。
            log::warn!("recent_events.json 解析失败({e}),已备份为 .bad 并清空");
            let _ = std::fs::rename(path, format!("{}.bad", path.display()));
            Vec::new()
        }
    }
}

/// 写「最新在前」的 Vec 到 `recent_events.json`。**不 panic**,失败 `log::warn`。
pub fn save(events: &[AgentEvent]) {
    if let Some(p) = path() {
        save_at(&p, events);
    }
}

/// 写到指定路径(测试 / 注入用)。
pub fn save_at(path: &Path, events: &[AgentEvent]) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("创建事件目录失败({e})");
            return;
        }
    }
    let text = match serde_json::to_string_pretty(events) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("序列化事件失败({e})");
            return;
        }
    };
    if let Err(e) = std::fs::write(path, text) {
        log::warn!("写入事件失败({e}): {}", path.display());
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
}
