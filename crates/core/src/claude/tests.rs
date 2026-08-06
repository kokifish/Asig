use super::*;
use crate::jsonl_tail::write_tmp;

/// signal_of mock:固定 signal、无消息内容(状态判定测试用)。
fn tail(signal: &str) -> Option<TailInfo> {
    Some(TailInfo {
        signal: signal.into(),
        last_user_msg: None,
        last_assistant_msg: None,
    })
}

fn pf(pid: u32, status: Option<&str>) -> SessionFile {
    pf_cwd(pid, status, None)
}

/// 带 cwd 的构造(不同 cwd = 不同会话,用以测试聚合边界)。
fn pf_cwd(pid: u32, status: Option<&str>, cwd: Option<&str>) -> SessionFile {
    SessionFile {
        pid,
        session_id: Some(format!("s{pid}")), // 有 session_id 才会触发 transcript 读取
        cwd: cwd.map(str::to_string),
        kind: None,
        status: status.map(str::to_string),
        status_updated_at: 0,
    }
}

// ---- classify:纯函数 ----

#[test]
fn classify_alive_maps_status() {
    // busy + 无 stop_reason(读不到 transcript)→ Working
    assert_eq!(
        classify(&pf(1, Some("busy")), None, true, None),
        Some(AgentStatus::Working)
    );
    // busy + tool_use → Working(正在跑工具)
    assert_eq!(
        classify(&pf(1, Some("busy")), None, true, Some("tool_use")),
        Some(AgentStatus::Working)
    );
    // busy + end_turn → NeedsDeci(等用户回)← bug 修复的核心
    assert_eq!(
        classify(&pf(1, Some("busy")), None, true, Some("end_turn")),
        Some(AgentStatus::NeedsDeci)
    );
    // busy + user(用户刚输入、Claude 处理中)→ Working(曾因残留 end_turn 误判 NeedsDeci)
    assert_eq!(
        classify(&pf(1, Some("busy")), None, true, Some("user")),
        Some(AgentStatus::Working)
    );
    // waiting(Claude 等用户输入/授权)→ NeedsDeci,优先于 transcript(尾部可能是历史 tool_use)
    assert_eq!(
        classify(&pf(1, Some("waiting")), None, true, Some("tool_use")),
        Some(AgentStatus::NeedsDeci)
    );
    assert_eq!(
        classify(&pf(1, Some("waiting")), None, true, None),
        Some(AgentStatus::NeedsDeci)
    );
    // idle → Done(stop_reason 无关;即 idle 优先于 stop_reason)
    assert_eq!(
        classify(&pf(1, Some("idle")), None, true, Some("end_turn")),
        Some(AgentStatus::Done)
    );
    assert_eq!(
        classify(&pf(1, Some("idle")), None, true, None),
        Some(AgentStatus::Done)
    );
    // shell(Claude REPL 模式,空闲等输入)→ Done,非 Working(曾误判运行中)
    assert_eq!(
        classify(&pf(1, Some("shell")), None, true, None),
        Some(AgentStatus::Done)
    );
    // status 未知 → Working
    assert_eq!(
        classify(&pf(1, None), None, true, None),
        Some(AgentStatus::Working)
    );
    assert_eq!(
        classify(&pf(1, Some("wat")), None, true, None),
        Some(AgentStatus::Working)
    );
}

#[test]
fn classify_dead_seen_before_is_offline() {
    // 曾见过(活的)→ 现在死了 = 失联
    assert_eq!(
        classify(
            &pf(1, Some("busy")),
            Some(AgentStatus::Working),
            false,
            None
        ),
        Some(AgentStatus::Offline)
    );
    assert_eq!(
        classify(&pf(1, Some("idle")), Some(AgentStatus::Done), false, None),
        Some(AgentStatus::Offline)
    );
    // 上一轮就已经是 Offline,文件还残留 → 继续 Offline
    assert_eq!(
        classify(
            &pf(1, Some("busy")),
            Some(AgentStatus::Offline),
            false,
            None
        ),
        Some(AgentStatus::Offline)
    );
}

// ---- read_tail_signal:transcript 尾部信号 ----

#[test]
fn read_tail_signal_user_after_end_turn_is_user() {
    // end_turn 后有 user(用户回了)→ "user"(Claude 处理中 → Working),不误判残留 end_turn
    let p = write_tmp(
        "user_after_end",
        &[
            r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
            r#"{"type":"user","message":{"role":"user"}}"#,
        ],
    );
    assert_eq!(read_tail(&p).unwrap().signal, "user");
    std::fs::remove_file(&p).ok();
}

#[test]
fn read_tail_signal_end_turn_when_last_is_end_turn() {
    // 最后是 assistant end_turn(等用户)→ "end_turn" → NeedsDeci
    let p = write_tmp(
        "end_last",
        &[
            r#"{"type":"user","message":{"role":"user"}}"#,
            r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
        ],
    );
    assert_eq!(read_tail(&p).unwrap().signal, "end_turn");
    std::fs::remove_file(&p).ok();
}

#[test]
fn read_tail_signal_tool_use_is_tool_use() {
    let p = write_tmp(
        "tool",
        &[r#"{"type":"assistant","message":{"stop_reason":"tool_use"}}"#],
    );
    assert_eq!(read_tail(&p).unwrap().signal, "tool_use");
    std::fs::remove_file(&p).ok();
}

#[test]
fn read_tail_extracts_messages() {
    // user 文本(string content)+ assistant 文本(数组 content blocks)+ signal 一次取出。
    let p = write_tmp(
        "msgs",
        &[
            r#"{"type":"user","message":{"role":"user","content":"修复 bug"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"已修复"}]}}"#,
        ],
    );
    let t = read_tail(&p).expect("应有 tail");
    assert_eq!(t.signal, "end_turn");
    assert_eq!(t.last_user_msg.as_deref(), Some("修复 bug"));
    assert_eq!(t.last_assistant_msg.as_deref(), Some("已修复"));
    std::fs::remove_file(&p).ok();
}

#[test]
fn classify_dead_never_seen_is_skipped() {
    // 古老残留 → None(不报)
    assert_eq!(classify(&pf(1, Some("busy")), None, false, None), None);
}

// ---- most_active:聚合活跃度 ----

#[test]
fn most_active_picks_busier() {
    use AgentStatus::*;
    assert_eq!(most_active(Some(Working), Done), Working);
    assert_eq!(most_active(Some(Done), Working), Working);
    assert_eq!(most_active(Some(NeedsDeci), Working), NeedsDeci);
    assert_eq!(most_active(Some(Working), NeedsDeci), NeedsDeci);
    assert_eq!(most_active(Some(Offline), Working), Working);
    assert_eq!(most_active(None, Done), Done);
}

// ---- discover_from:MOCK(is_alive / stop_reason / files / seen 全注入)----

#[test]
fn discover_healthy_working() {
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(100, Some("busy"))],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].status, AgentStatus::Working);
    assert_eq!(seen.get(&100), Some(&AgentStatus::Working));
}

#[test]
fn discover_busy_end_turn_is_needs_deci() {
    // busy 且 transcript 最后一条 end_turn → NeedsDeci(等用户)
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(100, Some("busy"))],
        &mut seen,
        |_| true,
        |_| tail("end_turn"),
        AgentKind::Claude,
    );
    assert_eq!(out[0].status, AgentStatus::NeedsDeci);
    assert_eq!(seen.get(&100), Some(&AgentStatus::NeedsDeci));
}

#[test]
fn discover_busy_tool_use_is_working() {
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(100, Some("busy"))],
        &mut seen,
        |_| true,
        |_| tail("tool_use"),
        AgentKind::Claude,
    );
    assert_eq!(out[0].status, AgentStatus::Working);
}

#[test]
fn discover_idle_reads_transcript_but_still_done() {
    // idle primary 也会读 transcript(取 last_assistant_msg 给 Done 事件),但 status 仍是
    // Done —— classify 对 idle 不看 signal(直接 Done)。
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(100, Some("idle"))],
        &mut seen,
        |_| true,
        |_| tail("end_turn"),
        AgentKind::Claude,
    );
    assert_eq!(out[0].status, AgentStatus::Done);
}

#[test]
fn discover_idle_primary_carries_assistant_msg() {
    // idle primary 读 transcript,last_assistant_msg 应能取到(Done 事件 content 来源)。
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(100, Some("idle"))],
        &mut seen,
        |_| true,
        |_| {
            Some(TailInfo {
                signal: "end_turn".into(),
                last_user_msg: None,
                last_assistant_msg: Some("完成了".into()),
            })
        },
        AgentKind::Claude,
    );
    assert_eq!(out[0].status, AgentStatus::Done);
    assert_eq!(out[0].last_assistant_msg.as_deref(), Some("完成了"));
}

#[test]
fn discover_dead_seen_before_becomes_offline() {
    // 上一轮见过 100 在 Working;本轮 pid 死了 → Offline
    let mut seen = HashMap::from([(100, AgentStatus::Working)]);
    let out = discover_from(
        &[pf(100, Some("busy"))],
        &mut seen,
        |_| false,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].status, AgentStatus::Offline);
    assert_eq!(seen.get(&100), Some(&AgentStatus::Offline));
}

#[test]
fn discover_ancient_leftover_is_ignored() {
    // 从没见过的死 pid 文件 → 不报,seen 也不记
    let mut seen = HashMap::new();
    let out = discover_from(
        &[pf(999, Some("busy"))],
        &mut seen,
        |_| false,
        |_| None,
        AgentKind::Claude,
    );
    assert!(out.is_empty());
    assert!(seen.is_empty());
}

#[test]
fn discover_offline_recovers_to_working() {
    // 曾 Offline;进程复活且 busy → Working
    let mut seen = HashMap::from([(300, AgentStatus::Offline)]);
    let out = discover_from(
        &[pf(300, Some("busy"))],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out[0].status, AgentStatus::Working);
    assert_eq!(seen.get(&300), Some(&AgentStatus::Working));
}

#[test]
fn discover_prunes_vanished_pids() {
    // 上轮见过 100、777;本轮只剩 100 的文件 → 777 被裁掉(干净退出)
    let mut seen = HashMap::from([(100, AgentStatus::Working), (777, AgentStatus::Done)]);
    let _ = discover_from(
        &[pf(100, Some("busy"))],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(seen.len(), 1);
    assert!(seen.contains_key(&100));
    assert!(!seen.contains_key(&777));
}

#[test]
fn discover_mixed_alive_and_dead() {
    // 不同目录 = 不同会话:100 活着 busy;200 上轮见过、现在死了 → 一 Working 一 Offline
    let mut seen = HashMap::from([(200, AgentStatus::Working)]);
    let out = discover_from(
        &[
            pf_cwd(100, Some("busy"), Some("/a")),
            pf_cwd(200, Some("busy"), Some("/b")),
        ],
        &mut seen,
        |pid| pid == 100,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].status, AgentStatus::Working);
    assert_eq!(out[1].status, AgentStatus::Offline);
}

// ---- 按 cwd 聚合(interactive + bg 子进程)----

#[test]
fn discover_bg_merges_into_interactive_same_cwd() {
    // 同 cwd:interactive + bg 合并为 1 个,主 = interactive(200);bg(100)不单独显示、不进 seen。
    let mut seen = HashMap::new();
    let mut bg = pf(100, Some("busy"));
    bg.kind = Some("bg".into());
    let out = discover_from(
        &[bg, pf(200, Some("busy"))],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1, "同 cwd 合并为 1 个");
    assert_eq!(out[0].native_id, "200", "主 = interactive");
    assert!(!seen.contains_key(&100), "bg pid 不进 seen");
    assert!(seen.contains_key(&200));
}

#[test]
fn discover_bg_busy_lifts_interactive_shell_to_working() {
    // fork 任务到后台跑的典型场景:interactive idle 成 shell(单独=Done)、bg busy(单独=Working)
    // → 同 cwd 聚合为 1 个,状态 = Working(取组内最活跃)← 本次 bug 的核心修复。
    let mut seen = HashMap::new();
    let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
    bg.kind = Some("bg".into());
    bg.session_id = None; // bg 即便没 sessionId 也贡献 busy 活跃度
    let mut inter = pf_cwd(200, Some("shell"), Some("/a"));
    inter.kind = Some("interactive".into());
    let out = discover_from(
        &[bg, inter],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].native_id, "200");
    assert_eq!(
        out[0].status,
        AgentStatus::Working,
        "bg busy 把 shell 主提升为 Working"
    );
    assert!(!seen.contains_key(&100));
    assert!(seen.contains_key(&200));
}

#[test]
fn discover_pure_bg_group_without_interactive_is_skipped() {
    // 组里只有 bg(无 interactive 主)→ 整组跳过(避免与 OpenClaw source 重叠)。
    let mut seen = HashMap::new();
    let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
    bg.kind = Some("bg".into());
    let out = discover_from(&[bg], &mut seen, |_| true, |_| None, AgentKind::Claude);
    assert!(out.is_empty());
    assert!(seen.is_empty());
}

#[test]
fn discover_distinct_cwd_are_distinct_sessions() {
    // 不同 cwd = 不同会话,各聚合成 1 个。
    let mut seen = HashMap::new();
    let out = discover_from(
        &[
            pf_cwd(100, Some("busy"), Some("/a")),
            pf_cwd(200, Some("busy"), Some("/b")),
        ],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].cwd.as_deref(), Some(Path::new("/a")));
    assert_eq!(out[1].cwd.as_deref(), Some(Path::new("/b")));
}

// ---- 多 interactive 同 cwd:取最新,旧的忽略(方案 A 修复)----

#[test]
fn discover_multiple_interactive_picks_newest_and_ignores_stale() {
    // 同 cwd 两个 interactive:旧的 busy+end_turn(单独会判 NeedsDeci)、新的 idle(最新)。
    // 修复前:most_active 合并 → NeedsDeci(旧污染)。
    // 修复后:primary = 最新 idle → Done;旧的 interactive 不参与合并。
    let mut old = pf_cwd(72955, Some("busy"), Some("/a"));
    old.status_updated_at = 1000;
    old.session_id = Some("old".into());
    let mut new = pf_cwd(61965, Some("idle"), Some("/a"));
    new.status_updated_at = 2000;
    let mut seen = HashMap::new();
    let out = discover_from(
        &[old, new],
        &mut seen,
        |_| true,
        |_| tail("end_turn"),
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1, "同 cwd 合并为 1 个");
    assert_eq!(out[0].native_id, "61965", "primary = 最新 interactive");
    assert_eq!(
        out[0].status,
        AgentStatus::Done,
        "新会话 idle → Done,不被旧 busy+end_turn 污染"
    );
    assert!(seen.contains_key(&61965));
    assert!(!seen.contains_key(&72955), "旧 interactive 不进 seen");
}

#[test]
fn discover_bg_still_merges_into_newest_interactive() {
    // 回归:bg 子进程的 busy 活跃度仍合并进最新 interactive 主(不破坏 fork 后台任务语义)。
    // 同 cwd:旧 interactive idle、新 interactive idle(=primary)、bg busy。
    let mut old = pf_cwd(72955, Some("idle"), Some("/a"));
    old.status_updated_at = 1000;
    let mut new = pf_cwd(61965, Some("idle"), Some("/a"));
    new.status_updated_at = 2000;
    let mut bg = pf_cwd(100, Some("busy"), Some("/a"));
    bg.kind = Some("bg".into());
    bg.status_updated_at = 1500;
    let mut seen = HashMap::new();
    let out = discover_from(
        &[old, new, bg],
        &mut seen,
        |_| true,
        |_| None,
        AgentKind::Claude,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].native_id, "61965", "primary = 最新 interactive");
    assert_eq!(
        out[0].status,
        AgentStatus::Working,
        "bg busy 合并进 primary → Working"
    );
    assert!(seen.contains_key(&61965));
    assert!(!seen.contains_key(&72955));
    assert!(!seen.contains_key(&100), "bg 不进 seen");
}

#[test]
fn session_file_parses_camelcase_and_kind() {
    // 实测 session 文件是 camelCase + 含 kind/sessionId;rename_all 让 session_id 读到
    // (NeedsDeci 的 transcript 读取前提),kind 用以区分 interactive / bg 子 claude。
    let json = r#"{"pid":123,"sessionId":"abc","cwd":"/x","kind":"bg","status":"shell"}"#;
    let f: SessionFile = serde_json::from_str(json).unwrap();
    assert_eq!(f.pid, 123);
    assert_eq!(f.session_id.as_deref(), Some("abc"));
    assert_eq!(f.cwd.as_deref(), Some("/x"));
    assert_eq!(f.kind.as_deref(), Some("bg"));
    assert_eq!(f.status.as_deref(), Some("shell"));
}
