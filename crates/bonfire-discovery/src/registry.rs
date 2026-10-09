//! 注册表：读取平台信息、判断存档键是否存在、导出/导入 .reg（文档 4.2 约定）。

use std::path::Path;

/// 把规则库中的注册表路径（`HKEY_CURRENT_USER/Software/X`）转为 reg.exe 使用的形式（`HKCU\Software\X`）。
pub fn to_reg_path(key: &str) -> Option<String> {
    let key = key.replace('/', "\\");
    let (hive, rest) = key.split_once('\\').unwrap_or((key.as_str(), ""));
    let short = match hive.to_ascii_uppercase().as_str() {
        "HKEY_CURRENT_USER" | "HKCU" => "HKCU",
        "HKEY_LOCAL_MACHINE" | "HKLM" => "HKLM",
        _ => return None,
    };
    Some(if rest.is_empty() { short.to_string() } else { format!("{short}\\{rest}") })
}

#[cfg(windows)]
mod imp {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    fn split(path: &str) -> Option<(RegKey, String)> {
        let full = super::to_reg_path(path)?;
        let (hive, rest) = full.split_once('\\').unwrap_or((full.as_str(), ""));
        let root = match hive {
            "HKCU" => RegKey::predef(HKEY_CURRENT_USER),
            _ => RegKey::predef(HKEY_LOCAL_MACHINE),
        };
        Some((root, rest.to_string()))
    }

    pub fn read_string(path: &str, value: &str) -> Option<String> {
        let (root, sub) = split(path)?;
        root.open_subkey_with_flags(sub, KEY_READ).ok()?.get_value::<String, _>(value).ok()
    }

    pub fn key_exists(path: &str) -> bool {
        split(path).is_some_and(|(root, sub)| root.open_subkey_with_flags(sub, KEY_READ).is_ok())
    }

    /// 子键名列表。
    pub fn subkeys(path: &str) -> Vec<String> {
        let Some((root, sub)) = split(path) else { return Vec::new() };
        match root.open_subkey_with_flags(sub, KEY_READ) {
            Ok(k) => k.enum_keys().flatten().collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn read_string(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn key_exists(_: &str) -> bool {
        false
    }
    pub fn subkeys(_: &str) -> Vec<String> {
        Vec::new()
    }
}

pub use imp::{key_exists, read_string, subkeys};

/// 用 reg.exe 导出注册表键到 .reg 文件（UTF-16）。
pub fn export(key: &str, dest: &Path) -> std::io::Result<()> {
    let reg = to_reg_path(key).ok_or_else(|| std::io::Error::other(format!("不支持的注册表路径 {key}")))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    run_reg(&["export", &reg, &dest.to_string_lossy(), "/y"])
}

/// 用 reg.exe 导入 .reg 文件。
pub fn import(file: &Path) -> std::io::Result<()> {
    run_reg(&["import", &file.to_string_lossy()])
}

fn run_reg(args: &[&str]) -> std::io::Result<()> {
    let out = std::process::Command::new("reg").args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("reg {} 失败：{}", args[0], String::from_utf8_lossy(&out.stderr).trim())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_paths() {
        assert_eq!(to_reg_path("HKEY_CURRENT_USER/Software/A B/C").as_deref(), Some(r"HKCU\Software\A B\C"));
        assert_eq!(to_reg_path(r"HKLM\SOFTWARE\x").as_deref(), Some(r"HKLM\SOFTWARE\x"));
        assert_eq!(to_reg_path("HKEY_USERS/x"), None);
    }

    #[test]
    #[cfg(windows)]
    fn export_import_roundtrip() {
        let key = format!("HKCU\\Software\\BonfireTest\\Reg{}", std::process::id());
        let ok = |args: &[&str]| assert!(std::process::Command::new("reg").args(args).output().unwrap().status.success(), "{args:?}");
        ok(&["add", &key, "/v", "Level", "/t", "REG_SZ", "/d", "boss", "/f"]);
        assert!(key_exists(&key));
        assert_eq!(read_string(&key, "Level").as_deref(), Some("boss"));
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("k.reg");
        export(&key, &file).unwrap();
        ok(&["add", &key, "/v", "Level", "/t", "REG_SZ", "/d", "changed", "/f"]);
        import(&file).unwrap();
        assert_eq!(read_string(&key, "Level").as_deref(), Some("boss"));
        ok(&["delete", &key, "/f"]);
        assert!(!key_exists(&key));
    }
}
