//! 交互式会话尾部信号解析:优先读 `agents/<id>/agent/openclaw-agent.sqlite`(新版
//! OpenClaw 的会话存储,jsonl 已停更仅存历史),无库 → 回退读 `agents/<id>/sessions/*.jsonl`
//! 末尾。两源事件同构(`event_json` 即原 jsonl 行),统一经 `signals_from_events` 计算:
//! 最后一条 message 的 role/stopReason、是否以 `leaf` 结尾、尾部是否含
//! sessions_yield/spawn 协调信号。

use crate::jsonl_tail;
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 一个 agent 最新交互式会话的尾部信号:mtime、最后一条 message 的 role 与 stop_reason、
/// 是否以 `leaf` 结尾、尾部是否含 sessions_yield/spawn 协调信号、最近 user/assistant 文本
/// (Panel start/done 事件展示用)。
#[derive(Clone)]
pub(super) struct SessionSignal {
    pub(super) mtime_ms: u64,
    pub(super) role: String,
    pub(super) stop: Option<String>,
    /// 文件最后一条事件是否为 `leaf`(OpenClaw 回合 marker;yield 循环每回合以 leaf 收尾)。
    pub(super) ends_with_leaf: bool,
    /// 尾部 6 条事件内是否含 `sessions_yield`/`sessions_spawn`(主 agent 在协调后台子 agent)。
    pub(super) coordinating: bool,
    /// 最近一条 user 消息文本(Panel start 事件)。None = 无 / 取不到。
    pub(super) last_user_msg: Option<String>,
    /// 最近一条 assistant 文本回复(Panel done 事件;跳过纯 toolCall)。None = 无 / 取不到。
    pub(super) last_assistant_msg: Option<String>,
}

/// 交互式会话尾部判在跑:user(刚发)/ toolResult(工具结果,模型继续)/ stop='toolUse'(模型
/// 发工具调用,工具在执行)。三者都表示模型还会接着动 → Working。final assistant(纯文本
/// 回复,stop 非 toolUse)→ 等用户,不算在跑。
pub(super) fn session_running(role: &str, stop: Option<&str>) -> bool {
    role == "user" || role == "toolResult" || stop == Some("toolUse")
}

/// 读 jsonl 尾部(末 ~32KB)算出尾部信号(`mtime_ms` 由调用方传入)。文件打不开 /
/// 空文件 → 全默认空信号(role 空、无 stop、非 leaf、非协调),与历史行为一致。
fn read_tail_signals(path: &Path, mtime_ms: u64) -> SessionSignal {
    let events = jsonl_tail::read_tail_lines(path, 32_768).unwrap_or_default();
    signals_from_events(&events, mtime_ms)
}

/// 从正序事件列表算出尾部信号(sqlite/jsonl 两源共用)。空事件 → 全默认空信号。
fn signals_from_events(events: &[serde_json::Value], mtime_ms: u64) -> SessionSignal {
    // 最后一条事件是否为 `leaf`(OpenClaw 回合 marker)。
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

    // 反序遍历:记最后一条 message 的 (role, stopReason)、最新一条 user 文本、最新一条
    // **有 text** 的 assistant 文本(跳过纯 toolCall)。三者全拿到或遍历完即停。
    let mut last_msg: Option<(String, Option<String>)> = None;
    let mut last_user_msg: Option<String> = None;
    let mut last_assistant_msg: Option<String> = None;
    for v in events.iter().rev() {
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let r = msg.get("role").and_then(|x| x.as_str()).unwrap_or("");
        if last_msg.is_none() {
            last_msg = Some((
                r.to_string(),
                msg.get("stopReason")
                    .and_then(|s| s.as_str())
                    .map(String::from),
            ));
        }
        if last_user_msg.is_none() && r == "user" {
            last_user_msg = jsonl_tail::extract_text(msg.get("content"));
        }
        if last_assistant_msg.is_none() && r == "assistant" {
            if let Some(t) = jsonl_tail::extract_text(msg.get("content")) {
                last_assistant_msg = Some(t);
            }
        }
        if last_msg.is_some() && last_user_msg.is_some() && last_assistant_msg.is_some() {
            break;
        }
    }
    let (role, stop) = last_msg.unwrap_or_default();

    SessionSignal {
        mtime_ms,
        role,
        stop,
        ends_with_leaf,
        coordinating,
        last_user_msg,
        last_assistant_msg,
    }
}

/// sqlite 源单会话尾部事件条数:覆盖一个完整 yield 循环(协调信号只看末 6 条)。
const SQLITE_TAIL_EVENTS: i64 = 32;

/// per-agent 库尾部信号:`agents/<id>/agent/openclaw-agent.sqlite` 的 `transcript_events`。
/// 只读 **active** 事件(join `session_transcript_active_events`):rewind 会把旧事件移出
/// active 集,按全量尾部取会被已回退的事件带偏。库打不开 / 表缺失 / 无 active 事件 →
/// None(调用方回退 jsonl 源)。
fn read_sqlite_signals(agent_dir: &Path) -> Option<SessionSignal> {
    let conn = crate::sys::open_readonly(&agent_dir.join("agent").join("openclaw-agent.sqlite"))?;
    // 最新 active 事件所在会话(该 agent 最近活动的会话)+ 该事件时间。
    let (sid, mtime) = conn
        .query_row(
            "SELECT e.session_id, e.created_at
             FROM transcript_events e
             JOIN session_transcript_active_events a
               ON a.session_id = e.session_id AND a.event_seq = e.seq
             ORDER BY e.created_at DESC, e.seq DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .ok()?;
    // 该会话尾部 active 事件:seq 倒序取 N 条再翻回正序;单条 event_json 解析失败跳过
    //(与 jsonl 丢坏行一致)。
    let mut stmt = conn
        .prepare(
            "SELECT e.event_json
             FROM transcript_events e
             JOIN session_transcript_active_events a
               ON a.session_id = e.session_id AND a.event_seq = e.seq
             WHERE e.session_id = ?1
             ORDER BY e.seq DESC LIMIT ?2",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![sid, SQLITE_TAIL_EVENTS], |r| r.get::<_, String>(0))
        .ok()?;
    let mut events: Vec<serde_json::Value> = rows
        .flatten()
        .filter_map(|j| serde_json::from_str(&j).ok())
        .collect();
    events.reverse();
    Some(signals_from_events(
        &events,
        u64::try_from(mtime).unwrap_or(0),
    ))
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

/// `agents/<id>/` 目录名集合:注册行的「落地面」。`agent_databases` 里会有文件已删但
/// 注册行残留的幽灵(acpx 后端 agent 常见:claude/codex 的库被清理后注册行仍在,gateway
/// 每次重启还盲刷 `last_seen_at` → 永远满足 30 天窗口)。只有目录真实存在才算 agent。
pub(super) fn materialized_agents(root: &Path) -> HashSet<String> {
    std::fs::read_dir(root.join("agents"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// 扫 `agents/<aid>/`,每 agent 取尾部信号:优先 per-agent sqlite,打不开(旧版无库 /
/// schema 不兼容 / 空库)→ 回退扫 `agents/<aid>/sessions/*.jsonl` 取 mtime 最新会话的尾部。
/// 只看活跃会话文件(排除 `.trajectory.jsonl` / `.deleted` / `.bak` / `.reset` 等派生/历史)。
pub(super) fn latest_session_signals(root: &Path) -> HashMap<String, SessionSignal> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(root.join("agents")) else {
        return out;
    };
    for e in entries.flatten() {
        let aid = e.file_name().to_string_lossy().to_string();
        if let Some(sig) = read_sqlite_signals(&e.path()) {
            out.insert(aid, sig);
            continue;
        }
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
    use rusqlite::Connection;

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
        assert_eq!(sig.last_user_msg.as_deref(), Some("hi"));
        assert_eq!(sig.last_assistant_msg, None, "toolUse 无文本回复");
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
        assert_eq!(sig.last_user_msg, None);
        assert_eq!(sig.last_assistant_msg, None);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_extracts_last_messages() {
        // user 文本(字符串 content)+ assistant 文本(数组 content blocks)都应取出。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_msg_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"帮我写函数"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"text","text":"这是回复代码"}}]}}}}"#
        )
        .unwrap();
        drop(f);
        let sig = read_tail_signals(&p, 0);
        assert_eq!(sig.last_user_msg.as_deref(), Some("帮我写函数"));
        assert_eq!(sig.last_assistant_msg.as_deref(), Some("这是回复代码"));
        assert_eq!(sig.role, "assistant", "最后一条 message = assistant");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_skips_toolcall_only_assistant() {
        // 纯 toolCall assistant(无 text block)→ last_assistant_msg 跳过它往前找有 text 的。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_toolcall_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"text","text":"真正的回复"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","name":"bash"}}]}}}}"#
        )
        .unwrap();
        drop(f);
        let sig = read_tail_signals(&p, 0);
        assert_eq!(
            sig.last_assistant_msg.as_deref(),
            Some("真正的回复"),
            "跳过纯 toolCall,取最近有 text 的 assistant"
        );
        std::fs::remove_file(&p).ok();
    }

    // ===== sqlite 源 =====

    /// 测试 root:每次全新 temp 目录,结束清理。
    fn test_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("asig_openclaw_sqlite_{tag}_{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("agents")).unwrap();
        root
    }

    /// 在 `agents/<aid>/agent/` 建最小 per-agent 库(只含查询用到的表/列),按序插入
    /// `(seq, event_json, created_at_ms, active)` 事件;active=false 的行不进 active 索引
    /// (模拟 rewind 摘除)。
    fn agent_sqlite(root: &Path, aid: &str, rows: &[(i64, &str, i64, bool)]) {
        let dir = root.join("agents").join(aid).join("agent");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("openclaw-agent.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TABLE transcript_events (
                    session_id TEXT NOT NULL, seq INTEGER NOT NULL,
                    event_json TEXT NOT NULL, created_at INTEGER NOT NULL,
                    PRIMARY KEY(session_id, seq));
             CREATE TABLE session_transcript_active_events (
                    session_id TEXT NOT NULL, active_position INTEGER NOT NULL,
                    event_seq INTEGER NOT NULL, message_position INTEGER,
                    context_eligible INTEGER,
                    PRIMARY KEY(session_id, active_position));",
        )
        .unwrap();
        for (i, (seq, json, ts, active)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO transcript_events(session_id, seq, event_json, created_at)
                 VALUES('s1', ?1, ?2, ?3)",
                params![seq, json, ts],
            )
            .unwrap();
            if *active {
                conn.execute(
                    "INSERT INTO session_transcript_active_events(
                            session_id, active_position, event_seq, message_position,
                            context_eligible)
                     VALUES('s1', ?1, ?2, NULL, NULL)",
                    params![i as i64, seq],
                )
                .unwrap();
            }
        }
    }

    /// 在 `agents/<aid>/sessions/` 写一个 jsonl 会话文件(旧存储回退测试用)。
    fn agent_jsonl(root: &Path, aid: &str, lines: &[&str]) {
        use std::io::Write;
        let dir = root.join("agents").join(aid).join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("old.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        drop(f);
    }

    #[test]
    fn sqlite_source_preferred_over_jsonl() {
        // 同 agent 既有 sqlite(新)又有 jsonl(旧):取 sqlite —— jsonl 仅存历史,
        // 取它会永远看到过期尾部。
        let root = test_root("prefer");
        agent_sqlite(
            &root,
            "main",
            &[
                (
                    1,
                    r#"{"type":"message","message":{"role":"user","content":"新消息"}}"#,
                    2000,
                    true,
                ),
                (
                    2,
                    r#"{"type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","name":"bash"}]}}"#,
                    3000,
                    true,
                ),
            ],
        );
        agent_jsonl(
            &root,
            "main",
            &[
                r#"{"type":"message","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"旧回复"}]}}"#,
            ],
        );
        let sigs = latest_session_signals(&root);
        let sig = sigs.get("main").expect("应有 main 信号");
        assert_eq!(sig.role, "assistant");
        assert_eq!(sig.stop.as_deref(), Some("toolUse"));
        assert_eq!(
            sig.last_user_msg.as_deref(),
            Some("新消息"),
            "取 sqlite 的 user 文本"
        );
        assert_eq!(sig.last_assistant_msg, None, "纯 toolCall 无文本");
        assert_eq!(sig.mtime_ms, 3000, "mtime = 最新 active 事件 created_at");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sqlite_absent_falls_back_to_jsonl() {
        // 无 per-agent 库(旧版 openclaw)→ 回退 jsonl 尾部。
        let root = test_root("fallback");
        agent_jsonl(
            &root,
            "main",
            &[
                r#"{"type":"message","message":{"role":"user","content":"hi"}}"#,
                r#"{"type":"message","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"done"}]}}"#,
            ],
        );
        let sigs = latest_session_signals(&root);
        let sig = sigs.get("main").expect("回退应产生信号");
        assert_eq!(sig.role, "assistant");
        assert_eq!(sig.stop.as_deref(), Some("stop"));
        assert_eq!(sig.last_assistant_msg.as_deref(), Some("done"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sqlite_schema_mismatch_falls_back_to_jsonl() {
        // 库文件存在但无 transcript 表(schema 演进/损坏)→ 查询失败 → 回退 jsonl,
        // 而不是把该 agent 判成「无会话」。
        let root = test_root("schema");
        let dir = root.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&dir).unwrap();
        Connection::open(dir.join("openclaw-agent.sqlite"))
            .unwrap()
            .execute_batch("CREATE TABLE unrelated(x)")
            .unwrap();
        agent_jsonl(
            &root,
            "main",
            &[r#"{"type":"message","message":{"role":"user","content":"旧但唯一"}}"#],
        );
        let sigs = latest_session_signals(&root);
        let sig = sigs.get("main").expect("schema 不兼容应回退 jsonl");
        assert_eq!(sig.last_user_msg.as_deref(), Some("旧但唯一"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sqlite_rewound_events_excluded() {
        // seq3(最新,assistant toolUse)被 rewind 摘出 active → 尾部止于 seq2(user),
        // 否则会被已回退的事件带偏成 assistant/toolUse。
        let root = test_root("rewind");
        agent_sqlite(
            &root,
            "kotomi",
            &[
                (
                    1,
                    r#"{"type":"message","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"前回合"}}]}}"#,
                    1000,
                    true,
                ),
                (
                    2,
                    r#"{"type":"message","message":{"role":"user","content":"重问"}}"#,
                    2000,
                    true,
                ),
                (
                    3,
                    r#"{"type":"message","message":{"role":"assistant","stopReason":"toolUse"}}"#,
                    3000,
                    false,
                ),
            ],
        );
        let sigs = latest_session_signals(&root);
        let sig = sigs.get("kotomi").expect("应有信号");
        assert_eq!(sig.role, "user", "rewind 掉的事件不应进尾部");
        assert_eq!(sig.stop, None);
        assert_eq!(sig.mtime_ms, 2000, "mtime 取 active 尾部的最新事件");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sqlite_yield_leaf_coordinating() {
        // 协调态全链路经 sqlite 源:spawn → yield → assistant stop → leaf(全 active),
        // coordinating + ends_with_leaf 必须成立,否则派发子 agent 期间误判 Done。
        let root = test_root("yield");
        agent_sqlite(
            &root,
            "main",
            &[
                (
                    1,
                    r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"sessions_spawn"}]}}"#,
                    1000,
                    true,
                ),
                (
                    2,
                    r#"{"type":"custom_message","message":{"customType":"openclaw.sessions_yield"}}"#,
                    2000,
                    true,
                ),
                (
                    3,
                    r#"{"type":"message","message":{"role":"assistant","stopReason":"stop"}}"#,
                    3000,
                    true,
                ),
                (4, r#"{"type":"leaf"}"#, 4000, true),
            ],
        );
        let sigs = latest_session_signals(&root);
        let sig = sigs.get("main").expect("应有信号");
        assert!(sig.ends_with_leaf, "应以 leaf 结尾");
        assert!(sig.coordinating, "尾部应检出 sessions_yield/spawn");
        assert_eq!(sig.role, "assistant");
        assert_eq!(sig.stop.as_deref(), Some("stop"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sqlite_no_active_events_ignored() {
        // 库有事件但全被摘出 active(极端:全部 rewind)→ None → 无信号(不炸、不误报)。
        let root = test_root("noactive");
        agent_sqlite(
            &root,
            "main",
            &[(
                1,
                r#"{"type":"message","message":{"role":"user","content":"x"}}"#,
                1000,
                false,
            )],
        );
        let sigs = latest_session_signals(&root);
        assert!(!sigs.contains_key("main"), "无 active 事件不应产生信号");
        std::fs::remove_dir_all(&root).ok();
    }
}
