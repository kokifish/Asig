//! agent-light-core:工具无关、可移植的监控内核(无 UI、无 AppKit)。
//!
//! 设计目标:<60MB / <1% CPU;UI 无关;留跨平台口子。
//! app 壳只调 `Monitor::poll()` 得到 `Snapshot`,据此驱动灯。

pub mod aggregate;
pub mod claude;
pub mod config;
pub mod openclaw;
pub mod source;
pub mod status;

/// jsonl 尾部读取共用工具(claude/openclaw 复用,内部)。
pub(crate) mod jsonl_tail;

pub use config::{
    Anim, DONE_NOTIF_DURATION_DEFAULT_S, DONE_NOTIF_DURATION_MAX_S, DONE_NOTIF_DURATION_MIN_S,
    DOT_SIZE_DEFAULT_PX, DOT_SIZE_MAX_PX, DOT_SIZE_MIN_PX, GRADIENT_LAYERS_DEFAULT,
    GRADIENT_LAYERS_MAX, GRADIENT_LAYERS_MIN, Lang, LightPosition, Settings, StateStyle, StyleKey,
    Theme,
};
pub use source::{AgentKind, AgentSession, AgentSource};
pub use status::{AgentStatus, Color, LightAnim, transition};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
}

impl Snapshot {
    /// 压成字符串指纹:任一关键字段(全局态/done_notif/会话 id+status)变 → 指纹变。
    /// app 层 tick 据此跳过无变化的 render(省 CPU,压到 ~0%)。
    pub fn signature(&self) -> String {
        let mut s = format!("{:?}|{}|", self.global, self.done_notif);
        for sess in &self.sessions {
            s.push_str(&format!("{}:{:?};", sess.id, sess.status));
        }
        s
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
}

impl Default for Monitor {
    fn default() -> Self {
        Self::with_enabled(&AgentKind::IMPLEMENTED)
    }
}

impl Monitor {
    /// 按启用的 agent 列表构造 source —— General「监控的 Agent」下拉切换时重建 Monitor 用。
    /// Trae 暂未实现,即便在列表里也不装配。某工具没装(`new()` 返回 None)→ 自然跳过。
    pub fn with_enabled(kinds: &[AgentKind]) -> Self {
        let mut sources: Vec<Box<dyn AgentSource>> = Vec::new();
        if kinds.contains(&AgentKind::Claude) {
            if let Some(s) = claude::ClaudeLikeSource::claude() {
                sources.push(Box::new(s));
            }
        }
        if kinds.contains(&AgentKind::CodeBuddy) {
            if let Some(s) = claude::ClaudeLikeSource::codebuddy() {
                sources.push(Box::new(s));
            }
        }
        if kinds.contains(&AgentKind::OpenClaw) {
            if let Some(s) = openclaw::OpenClawSource::new() {
                sources.push(Box::new(s));
            }
        }
        Self {
            sources,
            latched: RefCell::new(HashMap::new()),
            prev_global: RefCell::new(AgentStatus::Done),
            done_since: RefCell::new(None),
        }
    }

    /// 用给定 source 构造(测试用;生产走 `Default`)。
    pub fn with_sources(sources: Vec<Box<dyn AgentSource>>) -> Self {
        Self {
            sources,
            latched: RefCell::new(HashMap::new()),
            prev_global: RefCell::new(AgentStatus::Done),
            done_since: RefCell::new(None),
        }
    }

    /// 扫描所有 source,跑 sticky 状态机,返回快照。`done_notif_duration` = Done-Notification
    /// 窗口时长(app 层从设置喂入;内核不持有用户设置,保持纯净)。
    pub fn poll(&self, done_notif_duration: Duration) -> Snapshot {
        // 1) 收集本轮原始观测
        let mut raw: Vec<AgentSession> = Vec::new();
        for src in &self.sources {
            raw.extend(src.discover());
        }

        // 2) 状态机:叠加到 latched,得到本会话当前状态
        let mut latched = self.latched.borrow_mut();
        let mut sessions: Vec<AgentSession> = Vec::with_capacity(raw.len());
        for mut s in raw {
            let prev = latched
                .get(&s.id)
                .map(|l| l.status)
                .unwrap_or(AgentStatus::Done);
            let new = transition(prev, s.status);
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

        // 3) 剪掉本轮没出现的会话(进程/文件已消失)—— 但给锁定态 LATCH_GRACE 轮宽限:
        //    会话文件原子替换/瞬时改名时 live 会短暂不含它,立即删会让下轮重现以 Done 为
        //    基线,丢失锁定态(违反 sticky「不因抖动清」)。连续超宽限才删,避免幻影堆积。
        let live: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        latched.retain(|id, l| {
            if live.contains(id.as_str()) {
                true
            } else {
                l.misses += 1;
                l.misses <= LATCH_GRACE
            }
        });
        drop(latched);

        // 4) 聚合全局灯态
        let global = aggregate::global_status(&sessions);

        // 5) Done Notification 边沿检测:全局态从「非 Done」转入「Done」时
        //    记下时刻;在随后 done_notif_duration 内 done_notif=true。离开 Done 即
        //    清零,下次再进重新计时。
        let now = Instant::now();
        let entered_done =
            { *self.prev_global.borrow() != AgentStatus::Done && global == AgentStatus::Done };
        {
            let mut ds = self.done_since.borrow_mut();
            if entered_done {
                *ds = Some(now);
            }
            if global != AgentStatus::Done {
                *ds = None;
            }
        }
        let done_notif = match *self.done_since.borrow() {
            Some(t) => global == AgentStatus::Done && now.duration_since(t) < done_notif_duration,
            None => false,
        };
        *self.prev_global.borrow_mut() = global;

        Snapshot {
            sessions,
            global,
            done_notif,
        }
    }

    /// 推荐轮询间隔。DEV.md Design:默认 3s。
    pub fn poll_interval() -> Duration {
        Duration::from_millis(3000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{AgentKind, AgentSession, AgentSource};
    use std::sync::Mutex;

    /// 脚本化 mock source:按调用顺序依次返回预设的会话集(末项之后恒返回末项)。
    struct ScriptedSource {
        kind: AgentKind,
        script: Vec<Vec<AgentStatus>>,
        call: Mutex<usize>,
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
                    project: None,
                    status: *st,
                    label: None,
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
        })]);
        for _ in 0..4 {
            let _ = m.poll(Duration::from_secs(30)); // 4 轮:Error + 3 轮空(misses 3>2 删)
        }
        let s = m.poll(Duration::from_secs(30)); // 重现 → prev=Done → NeedsDeci
        assert_eq!(s.global, AgentStatus::NeedsDeci);
    }
}
