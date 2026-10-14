//! agent-light-core:工具无关、可移植的监控内核(无 UI、无 AppKit)。
//!
//! 设计目标:<60MB / <1% CPU;UI 无关;留跨平台口子。
//! app 壳只调 `Monitor::poll()` 得到 `Snapshot`,据此驱动灯。

pub mod aggregate;
pub mod claude;
pub mod config;
pub mod events;
pub mod hermes;
pub mod openclaw;
pub mod source;
pub mod status;
pub mod zcode;

/// jsonl 尾部读取共用工具(claude/openclaw 复用,内部)。
pub(crate) mod jsonl_tail;

/// 跨模块 JSON 持久化(config / events 共用,内部)。
pub(crate) mod persist;

/// 跨 source 共享系统工具(pid 探测 / 当前时间 / 只读 sqlite)。
pub(crate) mod sys;

pub use config::{
    Anim, DONE_NOTIF_DURATION_DEFAULT_S, DONE_NOTIF_DURATION_MAX_S, DONE_NOTIF_DURATION_MIN_S,
    DOT_SIZE_DEFAULT_PX, DOT_SIZE_MAX_PX, DOT_SIZE_MIN_PX, GRADIENT_LAYERS_DEFAULT,
    GRADIENT_LAYERS_MAX, GRADIENT_LAYERS_MIN, Lang, LightPosition, Settings, StateStyle, StyleKey,
    Theme,
};
pub use events::{AgentEvent, EventKind};
pub use source::{AgentKind, AgentSession, AgentSource};
pub use status::{AgentStatus, Color, LightAnim, transition};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 一次轮询的快照。灯效由 app 层的 `Settings::light(&snap)` 决定(done_notif 优先),
/// 内核不内嵌渲染策略。
pub struct Snapshot {
    pub sessions: Vec<AgentSession>,
    /// 全局灯态(多会话聚合)。
    pub global: AgentStatus,
    /// 刚转入 Done 的 `done_notif_duration` 内为 true —— app 层据此用 Done-Notification
    /// 灯效覆盖 `global` 的默认灯效;过期或离开 Done 后回退 `global`。时长由 `poll()` 入参给。
    pub done_notif: bool,
    /// **各自**刚转入 Done(`done_notif_duration` 窗口内)的会话 id —— Panel 行级 🔵 判定用。
    /// 区别于全局 `done_notif`(任一会话完成即全局亮):闲置已久的 Done 会话不在此列,
    /// 不会跟着别的会话的完成通知一起「假刚完成」(2026-10-14 kotomi/main/munger 实测)。
    pub done_notif_ids: HashSet<String>,
    /// 最近 start/done 事件(最新在前),Drop-down Panel 事件列表展示。
    pub recent_events: Vec<AgentEvent>,
}

impl Snapshot {
    /// 压成字符串指纹:任一关键字段(全局态/done_notif/会话 id+status/行级蓝集合/事件列表)变
    /// → 指纹变。app 层 tick 据此跳过无变化的 render(省 CPU,压到 ~0%)。事件指纹不含 `at_ms`
    /// (否则每轮 poll 时钟变都触发 render);新事件入列才变。行级蓝集合排序后计入:窗口过期
    /// 逐会话回落 🔵→🟢 时无其他字段变化,不进指纹会漏 render(残留过期 🔵)。
    pub fn signature(&self) -> String {
        let sessions: String = self
            .sessions
            .iter()
            .map(|s| format!("{}:{:?};", s.id, s.status))
            .collect();
        let mut blue: Vec<&str> = self.done_notif_ids.iter().map(String::as_str).collect();
        blue.sort_unstable();
        let events: String = self
            .recent_events
            .iter()
            .map(|e| format!("{:?}|{}|{:?}|{};", e.kind, e.label, e.event_kind, e.content))
            .collect();
        format!(
            "{:?}|{}|{}|blue={}|ev={}",
            self.global,
            self.done_notif,
            sessions,
            blue.join(","),
            events
        )
    }
}

/// 一个会话的锁定态 + 连续未观测到的轮数(宽限防短暂消失清掉锁定态)。
struct Latched {
    status: AgentStatus,
    misses: u8,
}

/// 锁定态宽限:会话连续 `LATCH_GRACE` 轮未出现才清(覆盖文件原子替换/瞬时改名等抖动)。
const LATCH_GRACE: u8 = 2;

/// 监控引擎:持有一组 source + 每会话的锁定状态(sticky 状态机)。
pub struct Monitor {
    sources: Vec<Box<dyn AgentSource>>,
    /// session_id -> 锁定态 + 宽限计数。跨轮询保留,实现 sticky。
    latched: RefCell<HashMap<String, Latched>>,
    /// 上一轮的全局态。用于检测「转入 Done」的边沿。
    prev_global: RefCell<AgentStatus>,
    /// 最近一次「全局态转入 Done」的时刻。Done Notification 窗口期由 `poll()` 入参决定。
    done_since: RefCell<Option<Instant>>,
    /// 每会话「转入 Done」的时刻(行级 done-notif 窗口用;全局窗口见 `done_since`)。
    /// 离开 Done 即删;随 latched 裁剪清理,防幽灵积累。
    done_at: RefCell<HashMap<String, Instant>>,
    /// 最近 start/done 事件 buffer(front=最新,容量 `events::MAX_EVENTS`)。跨轮询保留,
    /// 边沿检测产出。持久化到 `events_path`。
    recent_events: RefCell<VecDeque<AgentEvent>>,
    /// 是否已完成首轮 poll(false = 首轮)。首轮只建 latched 基线、不发边沿事件:重启 /
    /// 设置切换重建 Monitor 后,latched 清空,在跑会话的 prev 从 Done 基线起会误发一条
    /// 重复 Start(面板同内容 start 两条,2026-10-14 实测)。真边沿从第二轮起照常。
    primed: Cell<bool>,
    /// 事件持久化路径(None = 默认 `~/Library/Application Support/Asig/recent_events.json`;
    /// Some = 测试注入,避免污染真实文件)。
    events_path: Option<PathBuf>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::with_enabled(&AgentKind::IMPLEMENTED)
    }
}

impl Monitor {
    /// 按启用的 agent 列表构造 —— General「监控的 Agent」切换时重建 Monitor 用。
    pub fn with_enabled(kinds: &[AgentKind]) -> Self {
        Self::new_with_sources(Self::build_sources(kinds), None)
    }

    /// 用给定 source 构造(测试用;生产走 `Default`)。
    pub fn with_sources(sources: Vec<Box<dyn AgentSource>>) -> Self {
        Self::new_with_sources(sources, None)
    }

    /// 字段初始化只此一处(`with_enabled` / `with_sources` 共用)。`events_path`:None = 默认
    /// `~/Library/Application Support/Asig/recent_events.json`;Some = 测试注入,避免污染真实文件。
    fn new_with_sources(sources: Vec<Box<dyn AgentSource>>, events_path: Option<PathBuf>) -> Self {
        let recent: VecDeque<AgentEvent> = match &events_path {
            Some(p) => events::load_at(p).into(),
            None => events::load().into(),
        };
        Self {
            sources,
            latched: RefCell::new(HashMap::new()),
            prev_global: RefCell::new(AgentStatus::Done),
            done_since: RefCell::new(None),
            done_at: RefCell::new(HashMap::new()),
            recent_events: RefCell::new(recent),
            primed: Cell::new(false),
            events_path,
        }
    }

    /// 按 agent 列表装配 source。CodeBuddy 暂不支持、Trae 暂未实现,即便在列表里也不装配;
    /// 某工具没装(`new()` 返回 None)→ 自然跳过。
    fn build_sources(kinds: &[AgentKind]) -> Vec<Box<dyn AgentSource>> {
        let mut sources: Vec<Box<dyn AgentSource>> = Vec::new();
        if kinds.contains(&AgentKind::Claude) {
            if let Some(s) = claude::ClaudeLikeSource::claude() {
                sources.push(Box::new(s));
            }
        }
        if kinds.contains(&AgentKind::OpenClaw) {
            if let Some(s) = openclaw::OpenClawSource::new() {
                sources.push(Box::new(s));
            }
        }
        if kinds.contains(&AgentKind::Hermes) {
            if let Some(s) = hermes::HermesSource::new() {
                sources.push(Box::new(s));
            }
        }
        if kinds.contains(&AgentKind::Zcode) {
            if let Some(s) = zcode::ZcodeSource::new() {
                sources.push(Box::new(s));
            }
        }
        sources
    }

    /// 扫描所有 source,跑 sticky 状态机,返回快照。`done_notif_duration` = Done-Notification
    /// 窗口时长(app 层从设置喂入;内核不持有用户设置,保持纯净)。
    pub fn poll(&self, done_notif_duration: Duration) -> Snapshot {
        // 1) 收集本轮原始观测
        let mut raw: Vec<AgentSession> = Vec::new();
        for src in &self.sources {
            raw.extend(src.discover());
        }
        // 2-3) sticky 状态机 + 宽限裁剪 + 边沿记事件(首轮只建基线,不发事件)
        let mut pushed: Vec<AgentEvent> = Vec::new();
        let emit_events = self.primed.get();
        let sessions = self.apply_state_machine(raw, emit_events, &mut pushed);
        // 4) 聚合全局灯态
        let global = aggregate::global_status(&sessions);
        // 5) Done Notification 边沿(全局窗口 + 行级窗口共用同一 duration / now)
        let now = Instant::now();
        let done_notif = self.detect_done_notif(global, now, done_notif_duration);
        let done_notif_ids: HashSet<String> = {
            let done_at = self.done_at.borrow();
            done_at
                .iter()
                .filter(|(_, t)| now.duration_since(**t) < done_notif_duration)
                .map(|(id, _)| id.clone())
                .collect()
        };
        // 6) 边沿事件入 buffer(front=最新)+ 落盘
        if !pushed.is_empty() {
            let mut buf = self.recent_events.borrow_mut();
            for e in pushed {
                buf.push_front(e);
                while buf.len() > events::MAX_EVENTS {
                    buf.pop_back();
                }
            }
            let to_save: Vec<AgentEvent> = buf.iter().cloned().collect();
            match &self.events_path {
                Some(p) => events::save_at(p, &to_save),
                None => events::save(&to_save),
            }
        }
        let recent_events = self.recent_events.borrow().iter().cloned().collect();
        self.primed.set(true);
        Snapshot {
            sessions,
            global,
            done_notif,
            done_notif_ids,
            recent_events,
        }
    }

    /// sticky 状态机:本轮观测叠加到 latched 锁定态;未出现的会话给 `LATCH_GRACE` 轮宽限
    /// 才裁(防文件原子替换/改名抖动清掉锁定态)。同时做 per-session 边沿检测(进 Working→
    /// Start 事件取 `last_user_msg`;进 Done→Done 事件取 `last_assistant_msg`),推入 `pushed`。
    /// `emit_events=false`(首轮 poll)只建基线:在跑会话的 prev 从 Done 基线起,放行会
    /// 误发重复 Start。
    fn apply_state_machine(
        &self,
        raw: Vec<AgentSession>,
        emit_events: bool,
        pushed: &mut Vec<AgentEvent>,
    ) -> Vec<AgentSession> {
        let mut latched = self.latched.borrow_mut();
        let mut done_at = self.done_at.borrow_mut();
        let tick = Instant::now();
        let now_ms = crate::sys::now_ms();
        let mut sessions: Vec<AgentSession> = Vec::with_capacity(raw.len());
        for mut s in raw {
            let prev = latched
                .get(&s.id)
                .map(|l| l.status)
                .unwrap_or(AgentStatus::Done);
            let new = transition(prev, s.status);
            // 行级 done-notif 计时:转入 Done 记时刻,离开 Done 即清(窗口判定见 poll)。
            if prev != AgentStatus::Done && new == AgentStatus::Done {
                done_at.insert(s.id.clone(), tick);
            } else if new != AgentStatus::Done {
                done_at.remove(&s.id);
            }
            // 边沿 → 记 Panel start/done 事件(对应消息内容非空才记;首轮只建基线)。
            if emit_events && prev != AgentStatus::Working && new == AgentStatus::Working {
                if let Some(c) = s.last_user_msg.as_ref() {
                    pushed.push(AgentEvent {
                        kind: s.kind,
                        label: s.display_label(),
                        event_kind: EventKind::Start,
                        content: events::truncate_for_display(c),
                        at_ms: now_ms,
                    });
                }
            }
            if emit_events && prev != AgentStatus::Done && new == AgentStatus::Done {
                if let Some(c) = s.last_assistant_msg.as_ref() {
                    pushed.push(AgentEvent {
                        kind: s.kind,
                        label: s.display_label(),
                        event_kind: EventKind::Done,
                        content: events::truncate_for_display(c),
                        at_ms: now_ms,
                    });
                }
            }
            latched.insert(
                s.id.clone(),
                Latched {
                    status: new,
                    misses: 0,
                },
            );
            s.status = new;
            sessions.push(s);
        }
        let live: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        latched.retain(|id, l| {
            if live.contains(id.as_str()) {
                true
            } else {
                l.misses += 1;
                l.misses <= LATCH_GRACE
            }
        });
        // 会话被裁出 latched(连续 LATCH_GRACE+1 轮消失)→ 行级计时同步清理。
        done_at.retain(|id, _| latched.contains_key(id));
        sessions
    }

    /// Done Notification 边沿(步骤 5):全局态从非 Done 转入 Done 时记时刻;随后
    /// `duration` 内 done_notif=true,离开 Done 即清零(下次再进重新计时)。
    fn detect_done_notif(&self, global: AgentStatus, now: Instant, duration: Duration) -> bool {
        let entered_done =
            *self.prev_global.borrow() != AgentStatus::Done && global == AgentStatus::Done;
        {
            let mut ds = self.done_since.borrow_mut();
            if entered_done {
                *ds = Some(now);
            }
            if global != AgentStatus::Done {
                *ds = None;
            }
        }
        let in_window = match *self.done_since.borrow() {
            Some(t) => global == AgentStatus::Done && now.duration_since(t) < duration,
            None => false,
        };
        *self.prev_global.borrow_mut() = global;
        in_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{AgentKind, AgentSession, AgentSource};
    use std::sync::Mutex;

    /// 唯一临时事件路径(注入 Monitor,避免读写真实 `recent_events.json`)。
    fn temp_events_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "asig_monitor_{tag}_{}_{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// 脚本化 mock source:按调用顺序依次返回预设的会话集(末项之后恒返回末项)。
    struct ScriptedSource {
        kind: AgentKind,
        script: Vec<Vec<AgentStatus>>,
        call: Mutex<usize>,
        /// 每会话填入的 last_user_msg(测 Start 事件边沿;None = 不记)。
        user_msg: Option<String>,
        assistant_msg: Option<String>,
    }

    impl AgentSource for ScriptedSource {
        fn kind(&self) -> AgentKind {
            self.kind
        }
        fn discover(&self) -> Vec<AgentSession> {
            let mut i = self.call.lock().unwrap();
            let idx = (*i).min(self.script.len().saturating_sub(1));
            *i = idx + 1;
            self.script[idx]
                .iter()
                .map(|st| AgentSession {
                    kind: self.kind,
                    id: format!("{:?}:0", self.kind),
                    native_id: "0".into(),
                    cwd: None,
                    status: *st,
                    label: None,
                    last_user_msg: self.user_msg.clone(),
                    last_assistant_msg: self.assistant_msg.clone(),
                })
                .collect()
        }
    }

    #[test]
    fn done_notif_edges_on_transition_into_done() {
        let m = Monitor::with_sources(vec![Box::new(ScriptedSource {
            kind: AgentKind::Claude,
            call: Mutex::new(0),
            script: vec![
                vec![AgentStatus::Working], // 起步:Working
                vec![AgentStatus::Done],    // 转入 Done → notif 应亮
                vec![AgentStatus::Done],    // 仍是 Done(30s 内)→ notif 仍亮
                vec![AgentStatus::Working], // 离开 Done → notif 灭
                vec![AgentStatus::Done],    // 再进 Done → notif 再亮
            ],
            user_msg: None,
            assistant_msg: None,
        })]);

        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Working);
        assert!(!s.done_notif);

        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Done);
        assert!(s.done_notif, "转入 Done 应触发 Done Notification");

        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Done);
        assert!(s.done_notif, "30s 窗口内继续 Done,notif 保持");

        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Working);
        assert!(!s.done_notif, "离开 Done 后 notif 应灭");

        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Done);
        assert!(s.done_notif, "再次转入 Done 应重新触发");
    }

    #[test]
    fn per_session_done_notif_only_tracks_just_finished() {
        // 2026-10-14 实测场景回归:zcode 会话完成触发全局完成通知(全局 done_notif=true、
        // 灯闪浅蓝)时,面板所有闲置已久的 Done 行(openclaw 各 agent)被一起刷成 🔵
        // 「假刚完成」。行级窗口必须只含**自己**刚转入 Done 的会话:
        //   - fresh:Working→Done(刚完成)→ 在 done_notif_ids;
        //   - stale:首轮即 Done(闲置已久,无边沿)→ 不在;
        //   - fresh 再转 Working → 行级计时即时移除。
        struct TwoSessionSource {
            a: Mutex<Vec<AgentStatus>>, // "fresh"
            b: Mutex<Vec<AgentStatus>>, // "stale"
        }
        impl AgentSource for TwoSessionSource {
            fn kind(&self) -> AgentKind {
                AgentKind::Zcode
            }
            fn discover(&self) -> Vec<AgentSession> {
                // 每轮取脚本首位(耗尽后恒返回末位),两会话各一列。
                let take = |v: &Mutex<Vec<AgentStatus>>, tag: &str| {
                    let mut i = v.lock().unwrap();
                    let st = if i.len() > 1 { i.remove(0) } else { i[0] };
                    AgentSession {
                        kind: AgentKind::Zcode,
                        id: format!("Zcode:{tag}"),
                        native_id: tag.into(),
                        cwd: None,
                        status: st,
                        label: Some(tag.into()),
                        last_user_msg: None,
                        last_assistant_msg: None,
                    }
                };
                vec![take(&self.a, "fresh"), take(&self.b, "stale")]
            }
        }
        let m = Monitor::with_sources(vec![Box::new(TwoSessionSource {
            a: Mutex::new(vec![
                AgentStatus::Working, // 起步在跑
                AgentStatus::Done,    // 刚完成 → 行级窗口
                AgentStatus::Done,    // 仍 Done(窗口内)
                AgentStatus::Working, // 再跑 → 行级计时清
            ]),
            b: Mutex::new(vec![AgentStatus::Done; 4]), // 一直闲置 Done,无边沿
        })]);
        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.global, AgentStatus::Working);
        assert!(!s.done_notif);
        assert!(s.done_notif_ids.is_empty());

        let s = m.poll(Duration::from_secs(30)); // fresh 完成,全局转 Done
        assert!(s.done_notif, "全局完成通知应亮(灯闪浅蓝)");
        assert_eq!(
            s.done_notif_ids,
            ["Zcode:fresh".to_string()].into(),
            "只含刚完成的 fresh;闲置已久的 stale 不应进窗口"
        );

        let s = m.poll(Duration::from_secs(30)); // 窗口内仍是 Done
        assert_eq!(
            s.done_notif_ids,
            ["Zcode:fresh".to_string()].into(),
            "窗口内成员保持"
        );

        let s = m.poll(Duration::from_secs(30)); // fresh 再跑 → 移除
        assert!(!s.done_notif_ids.contains("Zcode:fresh"));
        assert!(!s.done_notif_ids.contains("Zcode:stale"));
    }

    #[test]
    fn first_poll_builds_baseline_without_events() {
        // 重启 / 设置切换重建 Monitor 后在跑的会话:latched 空 → prev 从 Done 基线起,
        // 首轮若发边沿会误发一条重复 Start(2026-10-14 面板实测同内容 start 两条)。
        // 首轮只建基线;之后的真边照常。
        let path = temp_events_path("first_poll");
        let m = Monitor::new_with_sources(
            vec![Box::new(ScriptedSource {
                kind: AgentKind::Claude,
                call: Mutex::new(0),
                script: vec![
                    vec![AgentStatus::Working], // 起步即在跑(Asig 启动前已 Working)
                    vec![AgentStatus::Working], // 仍在跑 → 无边沿
                    vec![AgentStatus::Done],    // 真完成 → Done 事件
                ],
                user_msg: Some("umsg".into()),
                assistant_msg: Some("amsg".into()),
            })],
            Some(path),
        );
        let s = m.poll(Duration::from_secs(30));
        assert!(s.recent_events.is_empty(), "首轮不应发 Start");
        let s = m.poll(Duration::from_secs(30));
        assert!(s.recent_events.is_empty(), "Working→Working 无边沿");
        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.recent_events.len(), 1);
        assert_eq!(s.recent_events[0].event_kind, EventKind::Done);
    }

    #[test]
    fn start_fires_when_session_appears_after_startup() {
        // 运行期新出现的在跑会话是真边沿,仍要发 Start(不能因首轮抑制而漏)。
        let path = temp_events_path("late_appear");
        let m = Monitor::new_with_sources(
            vec![Box::new(ScriptedSource {
                kind: AgentKind::Claude,
                call: Mutex::new(0),
                script: vec![vec![], vec![AgentStatus::Working]],
                user_msg: Some("new task".into()),
                assistant_msg: None,
            })],
            Some(path),
        );
        let s = m.poll(Duration::from_secs(30));
        assert!(s.recent_events.is_empty());
        let s = m.poll(Duration::from_secs(30));
        assert_eq!(s.recent_events.len(), 1, "新会话进 Working 应发 Start");
        assert_eq!(s.recent_events[0].event_kind, EventKind::Start);
        assert_eq!(s.recent_events[0].content, "new task");
    }

    #[test]
    fn latched_grace_keeps_locked_across_brief_absence() {
        // 锁定 Error → 短暂消失 1 轮 → 重现报 NeedsDeci:宽限内 latched 保留 Error,
        // transition(Error, NeedsDeci) 保持 Error(sticky 正确)。无宽限则会降级 NeedsDeci。
        let m = Monitor::with_sources(vec![Box::new(ScriptedSource {
            kind: AgentKind::Claude,
            call: Mutex::new(0),
            script: vec![
                vec![AgentStatus::Error],
                vec![],
                vec![AgentStatus::NeedsDeci],
            ],
            user_msg: None,
            assistant_msg: None,
        })]);
        let _ = m.poll(Duration::from_secs(30));
        let _ = m.poll(Duration::from_secs(30)); // 消失(live 不含)→ misses=1,宽限保留
        let s = m.poll(Duration::from_secs(30)); // 重现 NeedsDeci → prev=Error → 保持 Error
        assert_eq!(s.global, AgentStatus::Error);
    }

    #[test]
    fn latched_grace_expires_after_n_rounds() {
        // 连续 LATCH_GRACE+1 轮消失 → latched 清 → 重现以 Done 基线,接受新观测。
        let m = Monitor::with_sources(vec![Box::new(ScriptedSource {
            kind: AgentKind::Claude,
            call: Mutex::new(0),
            script: vec![
                vec![AgentStatus::Error],
                vec![],
                vec![],
                vec![],
                vec![AgentStatus::NeedsDeci],
            ],
            user_msg: None,
            assistant_msg: None,
        })]);
        for _ in 0..4 {
            let _ = m.poll(Duration::from_secs(30)); // 4 轮:Error + 3 轮空(misses 3>2 删)
        }
        let s = m.poll(Duration::from_secs(30)); // 重现 → prev=Done → NeedsDeci
        assert_eq!(s.global, AgentStatus::NeedsDeci);
    }

    #[test]
    fn edge_events_on_working_done_transitions() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "asig_monitor_edge_{}_{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        let m = Monitor::new_with_sources(
            vec![Box::new(ScriptedSource {
                kind: AgentKind::Claude,
                call: Mutex::new(0),
                script: vec![
                    vec![AgentStatus::Done],
                    vec![AgentStatus::Working],
                    vec![AgentStatus::Done],
                ],
                user_msg: Some("umsg".into()),
                assistant_msg: Some("amsg".into()),
            })],
            Some(path.clone()),
        );
        let _ = m.poll(Duration::from_secs(30)); // Done 基线,无事件
        let _ = m.poll(Duration::from_secs(30)); // → Working,push Start
        let snap = m.poll(Duration::from_secs(30)); // → Done,push Done
        assert!(snap.recent_events.len() >= 2);
        assert_eq!(
            snap.recent_events[0].event_kind,
            EventKind::Done,
            "最新在前 = Done"
        );
        assert_eq!(snap.recent_events[1].event_kind, EventKind::Start);
        assert_eq!(snap.recent_events[0].content, "amsg");
        assert_eq!(snap.recent_events[1].content, "umsg");
        assert_eq!(snap.recent_events[0].kind, AgentKind::Claude);
        // 落盘后新 Monitor 实例能加载回来(跨实例保留)
        let m2 = Monitor::new_with_sources(vec![], Some(path.clone()));
        let snap2 = m2.poll(Duration::from_secs(30));
        assert_eq!(snap2.recent_events.len(), 2, "落盘事件应能加载回来");
        assert_eq!(snap2.recent_events[0].event_kind, EventKind::Done);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn edge_events_capped_at_max() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "asig_monitor_max_{}_{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        // 交替 Working↔Done(21 项),每 poll 一个边沿事件;最终被容量上限裁到 MAX_EVENTS。
        let script: Vec<Vec<AgentStatus>> = (0..21)
            .map(|i| {
                vec![if i % 2 == 0 {
                    AgentStatus::Working
                } else {
                    AgentStatus::Done
                }]
            })
            .collect();
        let m = Monitor::new_with_sources(
            vec![Box::new(ScriptedSource {
                kind: AgentKind::Claude,
                call: Mutex::new(0),
                script,
                user_msg: Some("u".into()),
                assistant_msg: Some("a".into()),
            })],
            Some(path.clone()),
        );
        for _ in 0..21 {
            let _ = m.poll(Duration::from_secs(30));
        }
        let snap = m.poll(Duration::from_secs(30));
        assert_eq!(snap.recent_events.len(), events::MAX_EVENTS);
        let _ = std::fs::remove_file(&path);
    }
}
