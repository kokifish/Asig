//! jsonl 尾部读取共用工具。
//!
//! `claude.rs`(read_tail_signal)与 `openclaw.rs`(read_tail_signals)都要「读 jsonl
//! 文件末尾 N 字节、丢首行、逐行解析为事件」,仅提取的字段不同。抽此模块消除重复,并让
//! 尾部边界处理(首行截断丢弃、lossy 解码、跳过空行/解析失败)只在一处测试。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 读 `path` 末尾 `tail_bytes` 字节,丢首行(尾部起点多半落在行中间,首行被截断),
/// 逐行解析为 `serde_json::Value`(按文件顺序,跳过空行/解析失败)。
///
/// - 文件打不开 / 读失败 → `None`(上游据回退);
/// - 空文件 / 尾部无完整行 → `Some(vec![])`。
pub(crate) fn read_tail_lines(path: &Path, tail_bytes: u64) -> Option<Vec<serde_json::Value>> {
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let start = size.saturating_sub(tail_bytes);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // 起点非文件首(start>0)→ 首行多半被截断,跳过。
    let skip_first = if start > 0 { 1 } else { 0 };
    Some(
        text.lines()
            .skip(skip_first)
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect(),
    )
}

/// 测试 helper:在 temp 目录写一个 jsonl 文件,返回路径。jsonl_tail / claude 测试共用。
#[cfg(test)]
pub(crate) fn write_tmp(name: &str, lines: &[&str]) -> std::path::PathBuf {
    use std::io::Write;
    let p = std::env::temp_dir().join(format!("asig_test_{name}_{}.jsonl", std::process::id()));
    let mut f = std::fs::File::create(&p).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    drop(f);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_in_order() {
        let p = write_tmp("order", &[r#"{"type":"a","i":1}"#, r#"{"type":"b","i":2}"#]);
        let ev = read_tail_lines(&p, 4096).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].get("i").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(ev[1].get("type").and_then(|v| v.as_str()), Some("b"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn skips_empty_and_invalid_lines() {
        let p = write_tmp(
            "skip",
            &[r#"{"type":"ok"}"#, "", "not json", r#"{"type":"ok2"}"#],
        );
        let ev = read_tail_lines(&p, 4096).unwrap();
        assert_eq!(ev.len(), 2); // 只剩两条合法 json
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn none_when_file_missing() {
        assert!(
            read_tail_lines(
                std::path::Path::new("/nonexistent/asig_jsonl_tail_xyz"),
                4096,
            )
            .is_none()
        );
    }

    #[test]
    fn empty_file_returns_empty_vec() {
        let p = write_tmp("empty", &[]);
        let ev = read_tail_lines(&p, 4096).unwrap();
        assert!(ev.is_empty());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn drops_first_line_when_tail_starts_mid_file() {
        // 大文件 + 小 tail:start>0 → 首行被丢。写超 tail 的文件,验证末尾完整行被解析。
        use std::io::Write;
        let p =
            std::env::temp_dir().join(format!("asig_jsonl_tail_big_{}.jsonl", std::process::id()));
        let mut f = std::fs::File::create(&p).unwrap();
        let pad = "x".repeat(100);
        writeln!(f, r#"{{"pad":"{pad}"}}"#).unwrap();
        writeln!(f, r#"{{"i":1}}"#).unwrap();
        writeln!(f, r#"{{"i":2}}"#).unwrap();
        drop(f);
        let ev = read_tail_lines(&p, 60).unwrap();
        // 末尾完整行(i:2)必在;首行 padded 被(可能)截断丢弃。
        assert!(
            ev.iter()
                .any(|v| v.get("i").and_then(|x| x.as_i64()) == Some(2))
        );
        std::fs::remove_file(&p).ok();
    }
}
