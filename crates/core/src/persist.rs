//! 跨模块 JSON 持久化工具(config / events 共用)。
//!
//! 统一语义、**绝不 panic**:无文件 → 默认;权限/磁盘 IO 错 → `log::warn` + 默认;
//! JSON 损坏 → 备份成 `<path>.bad` + 默认(避免下次还解析失败)。写采用 `<path>.tmp → rename`
//! 原子替换,避免强杀/断电写一半导致下次解析失败。

use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

/// 默认路径 `~/Library/Application Support/Asig/<name>`(跨平台经 `dirs::config_dir`)。
pub fn app_support_path(name: &str) -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Asig").join(name))
}

/// 加载 JSON 为 `T`;无文件 / IO 错 / 损坏均回 `T::default()`(不 panic)。
/// 损坏文件备份成 `<path>.bad`,避免下次还解析失败。
pub fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            log::warn!("读取失败({e}),使用默认值: {}", path.display());
            return T::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "解析失败({e}),已备份为 .bad 并使用默认值: {}",
                path.display()
            );
            let _ = std::fs::rename(path, format!("{}.bad", path.display()));
            T::default()
        }
    }
}

/// 原子写 JSON(pretty 序列化 + `<path>.tmp → rename`)。失败仅 `log::warn`,不 panic。
pub fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("创建目录失败({e}): {}", parent.display());
            return;
        }
    }
    let text = match serde_json::to_string_pretty(value) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("序列化失败({e}): {}", path.display());
            return;
        }
    };
    // 原子替换:先写 <path>.tmp,rename 是 POSIX 原子操作,避免写一半损坏。
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if let Err(e) = std::fs::write(&tmp, &text) {
        log::warn!("写入失败({e}): {}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        log::warn!("替换文件失败({e}): {}", path.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
    struct Data {
        #[serde(default)]
        n: u32,
    }

    fn tmp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("asig_persist_{label}_{}.json", std::process::id()))
    }

    #[test]
    fn load_missing_is_default() {
        let p = tmp_path("missing");
        let _ = std::fs::remove_file(&p);
        assert_eq!(load_or_default::<Data>(&p), Data::default());
    }

    #[test]
    fn save_then_load_roundtrip_and_no_tmp_left() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        save_json(&p, &Data { n: 42 });
        assert_eq!(load_or_default::<Data>(&p), Data { n: 42 });
        // 原子写成功后 .tmp 应已被 rename 走,无残留。
        let mut tmp = p.as_os_str().to_os_string();
        tmp.push(".tmp");
        assert!(!PathBuf::from(&tmp).exists(), "tmp 残留");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_backed_up_and_default() {
        let dir = std::env::temp_dir().join(format!("asig_persist_corrupt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.json");
        std::fs::write(&p, "{ broken").unwrap();
        assert_eq!(load_or_default::<Data>(&p), Data::default());
        assert!(dir.join("f.json.bad").exists(), "应备份为 .bad");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
