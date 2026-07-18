//! 交互式会话尾部信号解析:读 `agents/<id>/sessions/*.jsonl` 末尾,提取最后一条 message
//! 的 role/stopReason、文件是否以 `leaf` 结尾、尾部是否含 sessions_yield/spawn 协调信号。

use crate::jsonl_tail;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个 agent 最新交互式会话的尾部信号(mtime + 最后一条 message 的 role + stop_reason
/// + 文件是否以 `leaf` 结尾 + 尾部是否含 sessions_yield/spawn 协调信号)。
#[derive(Clone)]
pub(super) struct SessionSignal {
    pub(super) mtime_ms: u64,
    pub(super) role: String,
    pub(super) stop: Option<String>,
    /// 文件最后一条事件是否为 `leaf`(OpenClaw 回合 marker;yield 循环每回合以 leaf 收尾)。
    pub(super) ends_with_leaf: bool,
    /// 尾部 6 条事件内是否含 `sessions_yield`/`sessions_spawn`(主 agent 在协调后台子 agent)。
    pub(super) coordinating: bool,
}

/// 交互式会话尾部判在跑:user(刚发)/ toolResult(工具结果,模型继续)/ stop='toolUse'(模型
/// 发工具调用,工具在执行)。三者都表示模型还会接着动 → Working。final assistant(纯文本
/// 回复,stop 非 toolUse)→ 等用户,不算在跑。
pub(super) fn session_running(role: &str, stop: Option<&str>) -> bool {
    role == "user" || role == "toolResult" || stop == Some("toolUse")
}

/// 读 jsonl 尾部(末 ~32KB),一次性算出尾部信号(`mtime_ms` 由调用方传入)。文件打不开 /
/// 空文件 → 全默认空信号(role 空、无 stop、非 leaf、非协调),与历史行为一致。
fn read_tail_signals(path: &Path, mtime_ms: u64) -> SessionSignal {
    let events = jsonl_tail::read_tail_lines(path, 32_768).unwrap_or_default();

    // 文件最后一条事件是否为 `leaf`(OpenClaw 回合 marker)。
    let ends_with_leaf = events
        .last()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()))
        == Some("leaf");

    // 尾部 6 条事件内是否含「主 agent 协调后台子 agent」信号:
    //   - custom_message:`message.customType == "openclaw.sessions_yield"`
    //   - assistant 工具调用:`message.content[].toolCall.name ∈ {sessions_yield, sessions_spawn}`
    // (89c26b75 实测两种形式并存;倒序取末 6 条覆盖一个完整 yield 循环。)
    let coordinating = events.iter().rev().take(6).any(|v| {
        if v.get("message")
            .and_then(|m| m.get("customType"))
            .and_then(|c| c.as_str())
            == Some("openclaw.sessions_yield")
        {
            return true;
        }
        v.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .is_some_and(|content| {
                content.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("toolCall")
                        && matches!(
                            b.get("name").and_then(|n| n.as_str()),
                            Some("sessions_yield") | Some("sessions_spawn")
                        )
                })
            })
    });

    // 反序找最后一条 type=message → (role, stopReason);无 message 则 role 空。
    let (role, stop) = events
        .iter()
        .rev()
        .find_map(|v| {
            (v.get("type").and_then(|t| t.as_str()) == Some("message")).then(|| {
                let role = v
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                let stop = v
                    .get("message")
                    .and_then(|m| m.get("stopReason"))
                    .and_then(|s| s.as_str())
                    .map(String::from);
                (role, stop)
            })
        })
        .unwrap_or((String::new(), None));

    SessionSignal {
        mtime_ms,
        role,
        stop,
        ends_with_leaf,
        coordinating,
    }
}

/// 派生/历史会话文件后缀(非真实交互式会话):trajectory / deleted / bak / reset。
const SESSION_EXCLUDE: &[&str] = &[".trajectory.", ".deleted", ".bak", ".reset"];

/// 是否为真实交互式会话文件(以 `.jsonl` 结尾且非派生/历史后缀)。
fn is_active_session(name: &str) -> bool {
    name.ends_with(".jsonl") && !SESSION_EXCLUDE.iter().any(|x| name.contains(x))
}

/// 文件修改时间距 epoch 的毫秒数;取不到(文件消失 / 不支持 mtime / 时钟倒跳)→ None。
fn mtime_ms(path: &Path) -> Option<u64> {
    use std::time::UNIX_EPOCH;
    let m = path.metadata().ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// 扫 `agents/<aid>/sessions/*.jsonl`,每 agent 取 mtime 最新的会话尾部信号。
/// 只看活跃会话文件(排除 `.trajectory.jsonl` / `.deleted` / `.bak` / `.reset` 等派生/历史)。
pub(super) fn latest_session_signals(root: &Path) -> HashMap<String, SessionSignal> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(root.join("agents")) else {
        return out;
    };
    for e in entries.flatten() {
        let aid = e.file_name().to_string_lossy().to_string();
        let Ok(sess) = std::fs::read_dir(e.path().join("sessions")) else {
            continue;
        };
        let mut best: Option<(u64, PathBuf)> = None;
        for f in sess.flatten() {
            if !is_active_session(&f.file_name().to_string_lossy()) {
                continue;
            }
            let Some(mt) = mtime_ms(&f.path()) else {
                continue;
            };
            if best.as_ref().is_none_or(|(b, _)| mt > *b) {
                best = Some((mt, f.path()));
            }
        }
        if let Some((mt, path)) = best {
            out.insert(aid, read_tail_signals(&path, mt));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_tail_reads_stopreason() {
        // 端到端:写一个临时 jsonl,确认 read_tail_signals 读到 message.stopReason
        // (字段是 message.stopReason 驼峰,非顶层 stop_reason —— 读错会永远空 → 工具链误判)。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_tail_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"hi"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","stopReason":"toolUse"}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"custom","customType":"model-snapshot"}}"#).unwrap();
        drop(f);
        let sig = read_tail_signals(&p, 0);
        assert_eq!(sig.role, "assistant");
        assert_eq!(sig.stop.as_deref(), Some("toolUse"));
        assert!(!sig.ends_with_leaf, "末行是 custom,非 leaf");
        assert!(!sig.coordinating, "无 yield/spawn");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_detects_yield_leaf() {
        // 端到端:yield 中断态文件(sessions_spawn → sessions_yield → cache-ttl → assistant
        // stop="stop" → leaf),read_tail_signals 必须同时报 coordinating=true + ends_with_leaf=true,
        // 否则派发子 agent 期间会误判 Done。两种 yield 表达(custom_message + assistant toolCall)都测。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_yield_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","name":"sessions_spawn"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"custom_message","message":{{"customType":"openclaw.sessions_yield"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"custom","customType":"openclaw.cache-ttl"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","stopReason":"stop"}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"leaf"}}"#).unwrap();
        drop(f);
        let sig = read_tail_signals(&p, 0);
        assert_eq!(sig.role, "assistant");
        assert_eq!(sig.stop.as_deref(), Some("stop"));
        assert!(sig.ends_with_leaf, "应以 leaf 结尾");
        assert!(sig.coordinating, "尾部应检出 sessions_yield/spawn");
        std::fs::remove_file(&p).ok();
    }
}
