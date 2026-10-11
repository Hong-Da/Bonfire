//! 界面状态（U-38）：游戏库的筛选、图库的查看方式与折叠等界面上的选择，保存在数据目录的
//! `ui-state.json` 中（与 `config.toml` 同目录）。由后台同步写入磁盘，退出程序前的最后一次修改也不会丢；
//! 不是配置，文件损坏或缺失时界面回到默认。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{AppError, Result};
use crate::paths::AppPaths;

/// 同时写入时串行，避免互相覆盖。
static WRITE: Mutex<()> = Mutex::new(());

/// 界面状态文件的位置。
pub fn ui_state_path(paths: &AppPaths) -> PathBuf {
    paths.config_file.with_file_name("ui-state.json")
}

/// 读取全部界面状态（键 → 值，值为界面自己序列化的文本）；文件缺失或损坏时为空。
pub fn load_ui_state(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 设置一项（`value` 为 None 时删除），立即写入磁盘（先写临时文件再替换）。
pub fn set_ui_state(path: &Path, key: &str, value: Option<String>) -> Result<()> {
    let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let mut map = load_ui_state(path);
    match value {
        Some(v) => {
            if map.get(key) == Some(&v) {
                return Ok(());
            }
            map.insert(key.to_string(), v);
        }
        None => {
            if map.remove(key).is_none() {
                return Ok(());
            }
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
    }
    let text = serde_json::to_string_pretty(&map).expect("界面状态可以序列化");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| AppError::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| AppError::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_load_and_remove() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("data/ui-state.json");
        assert!(load_ui_state(&p).is_empty());
        set_ui_state(
            &p,
            "bonfire.library.filters",
            Some(r#"{"modeFilter":"shots"}"#.into()),
        )
        .unwrap();
        set_ui_state(&p, "bonfire.gallery", Some("{}".into())).unwrap();
        let m = load_ui_state(&p);
        assert_eq!(
            m.get("bonfire.library.filters").unwrap(),
            r#"{"modeFilter":"shots"}"#
        );
        assert_eq!(m.len(), 2);
        set_ui_state(&p, "bonfire.gallery", None).unwrap();
        assert_eq!(load_ui_state(&p).len(), 1);
        // 损坏的文件按空处理，写入时重建。
        std::fs::write(&p, "not json").unwrap();
        assert!(load_ui_state(&p).is_empty());
        set_ui_state(&p, "k", Some("v".into())).unwrap();
        assert_eq!(load_ui_state(&p).get("k").unwrap(), "v");
    }
}
