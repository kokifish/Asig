//! 极简 logger:实现 `log::Log`,把 warn 及以上写 stderr(带 `Asig: ` 前缀)。
//! core 用 `log::warn!` 报诊断(设置读写失败、openclaw 打不开等),app 在此提供实现并初始化。
//! 不引入 env_logger / os_log:前者多一个依赖,后者需 objc2-foundation(破 core 零 AppKit)。

use log::{Level, LevelFilter, Log, Metadata, Record};

struct SimpleLogger;

impl Log for SimpleLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= Level::Warn
    }
    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            eprintln!("Asig: {}", r.args());
        }
    }
    fn flush(&self) {}
}

/// 装载全局 logger(只输出 warn 及以上到 stderr)。幂等 —— 重复调用无副作用。
pub fn init() {
    let _ = log::set_logger(Box::leak(Box::new(SimpleLogger)));
    log::set_max_level(LevelFilter::Warn);
}
