//! 读取被占用的存档文件（文档 §5，M5-3）。
//!
//! 游戏运行时可能以独占方式打开存档，普通读取会失败（共享冲突或区域锁定）。备份遇到这种文件时：
//! - 默认不备份这个游戏，提示文件被占用（自动备份推迟到下次）；
//! - 设置开启 `backup.use_vss` 时，用 Windows 卷影复制（VSS）读取：创建卷影副本需要管理员权限，
//!   每次会弹出 Windows 的用户账户控制确认。由一个以管理员身份运行的 PowerShell 脚本创建卷影副本、
//!   把被占用的文件复制到临时目录，再删除卷影副本；篝火只读取复制出来的文件，不碰存档目录。

use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};

/// 是否为“文件被其他程序占用”的错误（Windows：共享冲突 32、锁定冲突 33）。
pub fn is_locked(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32) | Some(33))
}

/// 从卷影副本复制被占用的文件。
pub trait ShadowReader: Send + Sync {
    /// 把 `files` 复制到 `out` 目录，返回与 `files` 一一对应的副本路径。
    fn copy(&self, files: &[PathBuf], out: &Path) -> Result<Vec<PathBuf>>;
}

/// 通过管理员身份运行的 PowerShell 使用 Win32_ShadowCopy。
pub struct ElevatedVss;

const SCRIPT: &str = r#"param([string]$List, [string]$Out, [string]$Result)
$ErrorActionPreference = 'Stop'
$shadows = @()
try {
  $files = [IO.File]::ReadAllLines($List, [Text.Encoding]::UTF8) | Where-Object { $_ -ne '' }
  $map = @{}
  foreach ($vol in ($files | ForEach-Object { [IO.Path]::GetPathRoot($_) } | Sort-Object -Unique)) {
    $r = (Get-WmiObject -List Win32_ShadowCopy).Create($vol, 'ClientAccessible')
    if ($r.ReturnValue -ne 0) { throw "创建卷影副本失败（$vol，错误 $($r.ReturnValue)）" }
    $s = Get-WmiObject Win32_ShadowCopy | Where-Object { $_.ID -eq $r.ShadowID }
    $shadows += $s
    $map[$vol] = $s.DeviceObject
  }
  $i = 0
  foreach ($f in $files) {
    $vol = [IO.Path]::GetPathRoot($f)
    $src = $map[$vol] + '\' + $f.Substring($vol.Length)
    [IO.File]::Copy($src, (Join-Path $Out ([string]$i)), $true)
    $i++
  }
  [IO.File]::WriteAllText($Result, 'ok', [Text.Encoding]::UTF8)
} catch {
  [IO.File]::WriteAllText($Result, "error: $($_.Exception.Message)", [Text.Encoding]::UTF8)
} finally {
  foreach ($s in $shadows) { try { $s.Delete() } catch {} }
}
"#;

impl ShadowReader for ElevatedVss {
    fn copy(&self, files: &[PathBuf], out: &Path) -> Result<Vec<PathBuf>> {
        if !cfg!(windows) {
            return Err(AppError::Invalid("卷影复制只在 Windows 上可用".into()));
        }
        std::fs::create_dir_all(out).map_err(|e| AppError::io(out, e))?;
        let list = out.join("files.txt");
        let script = out.join("vss-copy.ps1");
        let result = out.join("result.txt");
        let lines: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        std::fs::write(&list, lines.join("\r\n")).map_err(|e| AppError::io(&list, e))?;
        // PowerShell 5.1 读取中文脚本需要 BOM。
        let mut body = vec![0xEF, 0xBB, 0xBF];
        body.extend_from_slice(SCRIPT.as_bytes());
        std::fs::write(&script, body).map_err(|e| AppError::io(&script, e))?;
        // Start-Process 不会给含空格的参数加引号：每个路径自己带上双引号。
        let q = |p: &Path| format!("'\"{}\"'", p.to_string_lossy().replace('\'', "''"));
        let inner = format!(
            "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',{},{},{},{}",
            q(&script),
            q(&list),
            q(out),
            q(&result)
        );
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &inner]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let status = cmd.output().map_err(|e| AppError::io("powershell", e))?;
        let text = std::fs::read_to_string(&result).unwrap_or_default();
        let text = text.trim_start_matches('\u{feff}').trim();
        if text == "ok" {
            return Ok((0..files.len()).map(|i| out.join(i.to_string())).collect());
        }
        if text.is_empty() {
            let why = String::from_utf8_lossy(&status.stderr).trim().to_string();
            return Err(AppError::Invalid(format!(
                "没有获得管理员授权，无法用卷影复制读取被占用的文件{}",
                if why.is_empty() {
                    String::new()
                } else {
                    format!("（{why}）")
                }
            )));
        }
        Err(AppError::Invalid(format!(
            "卷影复制失败：{}",
            text.trim_start_matches("error: ")
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 测试用：按路径给出“卷影副本”中的内容。
    pub struct FakeShadow(pub HashMap<PathBuf, Vec<u8>>);

    impl ShadowReader for FakeShadow {
        fn copy(&self, files: &[PathBuf], out: &Path) -> Result<Vec<PathBuf>> {
            std::fs::create_dir_all(out).unwrap();
            let mut v = Vec::new();
            for (i, f) in files.iter().enumerate() {
                let p = out.join(i.to_string());
                std::fs::write(&p, self.0.get(f).expect("测试准备了内容")).unwrap();
                v.push(p);
            }
            Ok(v)
        }
    }

    #[test]
    fn locked_error_codes() {
        assert!(is_locked(&std::io::Error::from_raw_os_error(32)));
        assert!(is_locked(&std::io::Error::from_raw_os_error(33)));
        assert!(!is_locked(&std::io::Error::from_raw_os_error(5)));
    }

    /// 像游戏一样独占打开文件（不允许其他程序读取）。
    #[cfg(windows)]
    fn lock(p: &Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(p)
            .unwrap()
    }

    #[test]
    #[cfg(windows)]
    fn locked_files_block_backup_unless_vss_reads_them() {
        use crate::backup::{BackupOptions, BackupResult};
        let mut env = crate::testutil::Env::new();
        env.write("a.sav", b"one");
        env.write("b.sav", b"two");
        let g = env.add_game("g");
        let lock_repo = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock_repo, &g, &BackupOptions::default())
            .unwrap();
        env.write("b.sav", b"changed while playing");
        let held = lock(&env.saves().join("b.sav"));

        // 默认：提示文件被占用，不生成快照。
        let err = env
            .app
            .backup(&lock_repo, &g, &BackupOptions::default())
            .unwrap_err();
        assert!(
            matches!(&err, AppError::FilesLocked(v) if v.len() == 1),
            "{err}"
        );
        assert!(err.to_string().contains("被其他程序占用"));
        // 自动备份：推迟，不算出错。
        let r = env
            .app
            .run_auto_backup(&lock_repo, crate::auto::AutoKind::Scheduled)
            .unwrap();
        assert_eq!(r.deferred, [g.id.clone()]);
        assert!(r.items.iter().all(|i| i.error.is_none()));

        // 开启卷影复制：从副本读取被占用的文件。
        env.app.config.backup.use_vss = true;
        env.app.shadow = std::sync::Arc::new(FakeShadow(
            [(env.saves().join("b.sav"), b"changed while playing".to_vec())].into(),
        ));
        let r = env
            .app
            .backup(&lock_repo, &g, &BackupOptions::default())
            .unwrap();
        assert!(matches!(r, BackupResult::Created { .. }));
        drop(held);
        assert!(
            !env.app.status(&g).unwrap().summary.has_changes(),
            "备份的内容与磁盘一致"
        );
        assert!(
            !env.app
                .paths
                .cache_dir
                .join("vss")
                .read_dir()
                .is_ok_and(|mut d| d.next().is_some()),
            "临时副本已删除"
        );
    }
}
