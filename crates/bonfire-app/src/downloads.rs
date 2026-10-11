//! 导入下载的存档（U-42）：网上分享的存档通常是手动替换存档文件的方式，结构五花八门。
//!
//! - 来源：文件夹或 zip（含加密 zip，需要解压密码；zip 里的 zip 再解一层）。7z / rar 提示先解压。
//!   篝火导出的存档 / 篝火包识别出来，提示改用原来的导入。
//! - 递归扫描全部文件，忽略说明、图片、链接等；按文件名与当前存档（磁盘上的与最新快照）比对，
//!   找出“下载里的哪一层文件夹对应本机的哪个存档文件夹”，自动组合。
//! - 叠加（默认，下载只有部分文件时）：新快照 = 当前存档 + 下载的文件；整体替换：只有下载的文件。
//! - 存为一条新路线（默认“下载-日期”），不进主存档；不写入存档目录（需要时由界面接着恢复）。
//! - 解压密码只用于这次读取，不保存、不写日志。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use bonfire_core::index::GameRecord;
use bonfire_core::object::{Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
use bonfire_core::{ObjectId, RepoLock, scan};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::error::{AppError, Result};
use crate::util::{host_name, now_compact, now_local_rfc3339};

/// 来源的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadKind {
    /// 普通的下载存档，可以导入。
    Download,
    /// 篝火导出的存档文件或篝火包：用原来的导入。
    Bonfire,
    /// 7z / rar 等：请先解压。
    NeedsExtract,
    /// 加密的 zip：需要解压密码。
    NeedsPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadMode {
    /// 叠加到当前存档（其他文件保留）。
    Overlay,
    /// 整体替换（只有下载的文件）。
    Replace,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFile {
    /// 在下载中的相对路径（`/` 分隔）。
    pub rel: String,
    pub size: u64,
    /// 放到的位置：清单路径 `<存档位置序号>/<相对路径>`；对不上时为空。
    pub target: Option<String>,
    /// 说明、图片等，不导入。
    pub ignored: bool,
    /// 当前存档中有同一位置的文件（会被替换）。
    pub replaces: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLocation {
    pub ord: usize,
    /// 存档文件夹在磁盘上的位置。
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPreview {
    pub kind: DownloadKind,
    pub source_name: String,
    pub files: Vec<DownloadFile>,
    pub locations: Vec<DownloadLocation>,
    /// 建议的方式：下载覆盖了当前全部存档文件时整体替换，否则叠加。
    pub mode: DownloadMode,
    /// 当前存档的文件数。
    pub current_files: usize,
    /// 当前存档中已有的文件（清单路径，小写）：界面据此标出“覆盖现有文件 / 新增文件”，改了位置也能实时判断。
    pub existing: Vec<String>,
}

/// 导入时的选择：每个文件放到哪里（不导入的不列出）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadImport {
    pub mapping: Vec<(String, String)>,
    pub mode: DownloadMode,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    /// 新快照；与已有快照完全相同时为空（见 `identical`）。
    pub snapshot: Option<ObjectId>,
    pub identical: Option<ObjectId>,
    pub branch: Option<String>,
    pub files: usize,
}

/// 不导入的文件：说明、链接、图片、文档等。
fn is_ignored(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    const EXT: &[&str] = &[
        "txt", "md", "url", "html", "htm", "nfo", "pdf", "doc", "docx", "rtf", "jpg", "jpeg",
        "png", "gif", "webp", "bmp", "lnk", "mp4", "webm",
    ];
    EXT.contains(&ext)
        || name.contains("readme")
        || name.contains("说明")
        || name.contains("使用方法")
        || name == "desktop.ini"
        || name == "thumbs.db"
        || name == ".ds_store"
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// 把来源（文件夹或 zip）展开为 (相对路径, 文件)。
fn gather(
    src: &Path,
    staging: &Path,
    password: Option<&str>,
) -> Result<std::result::Result<Vec<(String, PathBuf)>, DownloadKind>> {
    let meta = std::fs::metadata(src).map_err(|e| AppError::io(src, e))?;
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !meta.is_dir() {
        match ext.as_str() {
            "bonfire" => return Ok(Err(DownloadKind::Bonfire)),
            "tar" | "gz" | "tgz" | "xz" | "bz2" => {
                return Ok(Err(DownloadKind::NeedsExtract));
            }
            _ => {}
        }
    }
    let root = if meta.is_dir() {
        src.to_path_buf()
    } else {
        // 7z 与 rar 也内置解压（U-45）；其余按 zip 读取。
        let r = match ext.as_str() {
            "7z" => extract_7z(src, staging, password)?,
            "rar" => extract_rar(src, staging, password)?,
            _ => extract_zip(src, staging, password)?,
        };
        match r {
            Some(kind) => return Ok(Err(kind)),
            None => staging.to_path_buf(),
        }
    };
    let mut out = Vec::new();
    collect(&root, &root, staging, password, 0, &mut out)?;
    if out.iter().any(|(rel, _)| {
        let n = rel.rsplit('/').next().unwrap_or(rel);
        n == crate::transfer::EXPORT_MANIFEST || n == "bundle.json" || n == "collection.json"
    }) {
        return Ok(Err(DownloadKind::Bonfire));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Ok(out))
}

/// 递归收集文件；zip 里的 zip 再解开（最多两层）。
fn collect(
    root: &Path,
    dir: &Path,
    staging: &Path,
    password: Option<&str>,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let rd = std::fs::read_dir(dir).map_err(|e| AppError::io(dir, e))?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(root, &p, staging, password, depth, out)?;
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let is_zip = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip"));
        if is_zip && depth < 2 {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let inner = staging
                .join(format!("nested-{depth}-{}", out.len()))
                .join(&stem);
            if extract_zip(&p, &inner, password)?.is_none() {
                let prefix = rel
                    .rsplit_once('/')
                    .map(|(d, _)| format!("{d}/"))
                    .unwrap_or_default();
                let mut nested = Vec::new();
                collect(&inner, &inner, staging, password, depth + 1, &mut nested)?;
                for (r, abs) in nested {
                    out.push((format!("{prefix}{stem}/{r}"), abs));
                }
                continue;
            }
        }
        out.push((rel, p));
    }
    Ok(())
}

/// 压缩包内的相对路径（`/` 或 `\` 分隔）转为安全的相对路径：拒绝盘符与 `..`，防止解压到临时文件夹外。
fn safe_rel(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if p.contains(':') => return None,
            p => out.push(p),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn bad_password() -> AppError {
    AppError::Invalid("解压密码不对，请重新输入".into())
}

/// 解压 7z 到 `dest`（纯 Rust，支持 AES 加密）。需要密码而没有给时返回 `NeedsPassword`。
fn extract_7z(src: &Path, dest: &Path, password: Option<&str>) -> Result<Option<DownloadKind>> {
    use sevenz_rust2::{Error as SzError, Password};
    let pw = password.filter(|p| !p.is_empty());
    let file = std::fs::File::open(src).map_err(|e| AppError::io(src, e))?;
    let res = sevenz_rust2::decompress_with_extract_fn_and_password(
        file,
        dest,
        pw.map(Password::from).unwrap_or_else(Password::empty),
        |entry, reader, _| {
            let Some(rel) = safe_rel(entry.name()) else {
                return Err(SzError::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("压缩包中有不安全的路径：{}", entry.name()),
                )));
            };
            let out = dest.join(rel);
            if entry.is_directory() {
                std::fs::create_dir_all(&out)?;
            } else {
                if let Some(dir) = out.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let mut f = std::fs::File::create(&out)?;
                std::io::copy(reader, &mut f)?;
            }
            Ok(true)
        },
    );
    match res {
        Ok(()) => Ok(None),
        Err(SzError::PasswordRequired) if pw.is_none() => Ok(Some(DownloadKind::NeedsPassword)),
        Err(SzError::PasswordRequired | SzError::MaybeBadPassword(_)) => Err(bad_password()),
        // 文件名也加密的 7z：密码不对时解出的目录是乱码，表现为“没有目录”等错误。
        Err(_) if pw.is_some() && seven_zip_needs_password(src) => Err(bad_password()),
        Err(e) => Err(AppError::Invalid(format!(
            "读取 7z 压缩包失败：{e}。如果压缩包没问题，可以先用解压工具解压，再导入解压出来的文件夹"
        ))),
    }
}

/// 这个 7z 是否需要密码（不带密码读取时要求密码）。只读取，不写文件。
fn seven_zip_needs_password(src: &Path) -> bool {
    let Ok(file) = std::fs::File::open(src) else {
        return false;
    };
    matches!(
        sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty()).and_then(
            |mut r| {
                r.for_each_entries(|_, reader| {
                    std::io::copy(reader, &mut std::io::sink())?;
                    Ok(true)
                })
            }
        ),
        Err(sevenz_rust2::Error::PasswordRequired)
    )
}

/// 解压 rar 到 `dest`（内置 RarLab 的 unRAR 解压代码，支持加密）。需要密码而没有给时返回 `NeedsPassword`。
fn extract_rar(src: &Path, dest: &Path, password: Option<&str>) -> Result<Option<DownloadKind>> {
    use unrar::error::Code;
    let pw = password.filter(|p| !p.is_empty());
    let map = |e: unrar::error::UnrarError| -> Result<Option<DownloadKind>> {
        match e.code {
            Code::MissingPassword if pw.is_none() => Ok(Some(DownloadKind::NeedsPassword)),
            Code::MissingPassword | Code::BadPassword | Code::BadData if pw.is_some() => {
                Err(bad_password())
            }
            _ => Err(AppError::Invalid(format!(
                "读取 rar 压缩包失败：{e}。如果压缩包没问题，可以先用解压工具解压，再导入解压出来的文件夹"
            ))),
        }
    };
    let archive = match pw {
        Some(p) => unrar::Archive::with_password(src, p.as_bytes()),
        None => unrar::Archive::new(src),
    };
    let mut cursor = match archive.open_for_processing() {
        Ok(c) => c,
        Err(e) => return map(e),
    };
    loop {
        let header = match cursor.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => return map(e),
        };
        let entry = header.entry();
        if entry.is_encrypted() && pw.is_none() {
            return Ok(Some(DownloadKind::NeedsPassword));
        }
        let step = if entry.is_file() {
            let name = entry.filename.to_string_lossy().into_owned();
            let Some(rel) = safe_rel(&name) else {
                return Err(AppError::Invalid(format!("压缩包中有不安全的路径：{name}")));
            };
            let out = dest.join(rel);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            header.extract_to(&out)
        } else {
            header.skip()
        };
        cursor = match step {
            Ok(c) => c,
            Err(e) => return map(e),
        };
    }
    Ok(None)
}

/// 解压 zip 到 `dest`。需要密码而没有给时返回 `NeedsPassword`；密码不对时报错。
fn extract_zip(src: &Path, dest: &Path, password: Option<&str>) -> Result<Option<DownloadKind>> {
    let file = std::fs::File::open(src).map_err(|e| AppError::io(src, e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| {
        AppError::Invalid(format!(
            "「{}」不是可以读取的 zip 压缩包；如果是 7z / rar，请先用解压工具解压，再导入解压出来的文件夹",
            src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ))
    })?;
    let encrypted =
        (0..zip.len()).any(|i| zip.by_index_raw(i).map(|f| f.encrypted()).unwrap_or(false));
    if encrypted && password.is_none_or(str::is_empty) {
        return Ok(Some(DownloadKind::NeedsPassword));
    }
    for i in 0..zip.len() {
        let mut entry = match (encrypted, password) {
            (true, Some(pw)) => zip.by_index_decrypt(i, pw.as_bytes()),
            _ => zip.by_index(i),
        }
        .map_err(|e| match e {
            zip::result::ZipError::InvalidPassword => {
                AppError::Invalid("解压密码不对，请重新输入".into())
            }
            other => AppError::Invalid(format!("读取压缩包失败：{other}")),
        })?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            return Err(AppError::Invalid(format!(
                "压缩包中有不安全的路径：{}",
                entry.name().unwrap_or_default()
            )));
        };
        let out = dest.join(rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| AppError::io(&out, e))?;
        // 密码错误（传统加密有时能通过校验）时内容校验失败。
        std::io::copy(&mut entry, &mut f).map_err(|e| {
            if encrypted {
                AppError::Invalid("解压密码不对，请重新输入".into())
            } else {
                AppError::io(&out, e)
            }
        })?;
        f.flush().map_err(|e| AppError::io(&out, e))?;
    }
    Ok(None)
}

/// 自动对应：用文件名与当前存档的文件比对，找出“下载里的文件夹 → 本机的存档文件夹”。
fn auto_map(files: &[(String, u64)], known: &[String], locations: usize) -> Vec<Option<String>> {
    let split = |p: &str| -> Vec<String> { p.split('/').map(str::to_string).collect() };
    let mut by_name: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for k in known {
        let parts = split(k);
        by_name
            .entry(lower(parts.last().map(String::as_str).unwrap_or("")))
            .or_default()
            .push(parts);
    }
    // 投票：(下载中的文件夹, 本机文件夹前缀) → 票数。
    let mut votes: HashMap<(String, String), usize> = HashMap::new();
    for (rel, _) in files.iter().filter(|(r, _)| !is_ignored(r)) {
        let parts = split(rel);
        let name = lower(parts.last().map(String::as_str).unwrap_or(""));
        for k in by_name.get(&name).into_iter().flatten() {
            // 末尾相同的路径段数（至少文件名）。
            let mut n = 0;
            while n < parts.len()
                && n + 1 < k.len()
                && lower(&parts[parts.len() - 1 - n]) == lower(&k[k.len() - 1 - n])
            {
                n += 1;
            }
            let dl = parts[..parts.len() - n].join("/");
            let kp = k[..k.len() - n].join("/");
            *votes.entry((dl, kp)).or_default() += 1;
        }
    }
    // 每个下载文件夹取票数最多的本机前缀。
    let mut best: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for ((dl, kp), n) in votes {
        let e = best.entry(dl).or_insert((kp.clone(), 0));
        if n > e.1 || (n == e.1 && kp < e.0) {
            *e = (kp, n);
        }
    }
    let prefixes: Vec<(String, String)> = best.into_iter().map(|(d, (k, _))| (d, k)).collect();
    // 没有任何同名文件、且只有一个存档位置：去掉外层的包裹文件夹，整个放进这个位置。
    let wrapper = if prefixes.is_empty() && locations == 1 {
        let dirs: Vec<Vec<String>> = files
            .iter()
            .filter(|(r, _)| !is_ignored(r))
            .map(|(r, _)| {
                let mut p = split(r);
                p.pop();
                p
            })
            .collect();
        let mut common: Vec<String> = dirs.first().cloned().unwrap_or_default();
        for d in &dirs {
            let n = common.iter().zip(d).take_while(|(a, b)| a == b).count();
            common.truncate(n);
        }
        Some(common.join("/"))
    } else {
        None
    };
    files
        .iter()
        .map(|(rel, _)| {
            if is_ignored(rel) {
                return None;
            }
            let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            // 最长的、是这个文件所在文件夹前缀的下载文件夹。
            let hit = prefixes
                .iter()
                .filter(|(d, _)| {
                    d.is_empty() || dir == d.as_str() || dir.starts_with(&format!("{d}/"))
                })
                .max_by_key(|(d, _)| d.len());
            if let Some((d, k)) = hit {
                let rest = if d.is_empty() {
                    rel.as_str()
                } else {
                    &rel[d.len() + 1..]
                };
                return Some(format!("{k}/{rest}"));
            }
            if let Some(w) = &wrapper {
                let rest = if w.is_empty() {
                    rel.as_str()
                } else {
                    rel.strip_prefix(&format!("{w}/")).unwrap_or(rel)
                };
                return Some(format!("0/{rest}"));
            }
            None
        })
        .collect()
}

/// 清单路径是否合法：`<序号>/<相对路径>`，序号在范围内，没有 `..`。
fn valid_target(t: &str, locations: usize) -> bool {
    let Some((ord, rest)) = t.split_once('/') else {
        return false;
    };
    ord.parse::<usize>().is_ok_and(|o| o < locations)
        && !rest.is_empty()
        && rest
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

impl App {
    fn download_staging(&self, tag: &str) -> PathBuf {
        self.paths
            .cache_dir
            .join("import")
            .join(format!("{tag}-{}", now_compact()))
    }

    /// 当前存档的文件：磁盘上的，加上最新快照中的（清单路径）。
    fn current_save_paths(&self, game: &GameRecord) -> Result<(Vec<String>, Vec<scan::DiskFile>)> {
        let (disk, _) = self.detect(game, false)?;
        let mut paths: HashSet<String> = disk.iter().map(|f| f.path.clone()).collect();
        if let Some(id) = self.repo.head_snapshot(&game.id)? {
            let tree = self.repo.get_tree(&self.repo.get_snapshot(&id)?.tree)?;
            paths.extend(tree.entries.into_iter().map(|e| e.path));
        }
        let mut paths: Vec<String> = paths.into_iter().collect();
        paths.sort();
        Ok((paths, disk))
    }

    /// 预览导入下载的存档。不写入任何东西（解压到临时文件夹，结束时删除）。
    pub fn inspect_download(
        &self,
        game: &GameRecord,
        src: &Path,
        password: Option<&str>,
    ) -> Result<DownloadPreview> {
        let staging = self.download_staging("download-inspect");
        let result = self.inspect_download_in(game, src, password, &staging);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn inspect_download_in(
        &self,
        game: &GameRecord,
        src: &Path,
        password: Option<&str>,
        staging: &Path,
    ) -> Result<DownloadPreview> {
        let locations = self.locations(&game.id)?;
        if locations.is_empty() {
            return Err(AppError::NoLocations(game.name.clone()));
        }
        let source_name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let locs: Vec<DownloadLocation> = locations
            .iter()
            .enumerate()
            .map(|(ord, l)| DownloadLocation {
                ord,
                path: l.resolved_path.clone(),
            })
            .collect();
        let files = match gather(src, staging, password)? {
            Ok(f) => f,
            Err(kind) => {
                return Ok(DownloadPreview {
                    kind,
                    source_name,
                    files: Vec::new(),
                    locations: locs,
                    mode: DownloadMode::Overlay,
                    current_files: 0,
                    existing: Vec::new(),
                });
            }
        };
        let (known, disk) = self.current_save_paths(game)?;
        let sized: Vec<(String, u64)> = files
            .iter()
            .map(|(rel, abs)| {
                (
                    rel.clone(),
                    std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0),
                )
            })
            .collect();
        let targets = auto_map(&sized, &known, locations.len());
        let known_set: HashSet<String> = known.iter().map(|k| lower(k)).collect();
        let out: Vec<DownloadFile> = sized
            .into_iter()
            .zip(targets)
            .map(|((rel, size), target)| DownloadFile {
                ignored: is_ignored(&rel),
                replaces: target
                    .as_ref()
                    .is_some_and(|t| known_set.contains(&lower(t))),
                rel,
                size,
                target,
            })
            .collect();
        let covered: HashSet<String> = out
            .iter()
            .filter_map(|f| f.target.as_ref().map(|t| lower(t)))
            .collect();
        let disk_paths: Vec<String> = disk.iter().map(|f| lower(&f.path)).collect();
        let mode = if !disk_paths.is_empty() && disk_paths.iter().all(|p| covered.contains(p)) {
            DownloadMode::Replace
        } else {
            DownloadMode::Overlay
        };
        Ok(DownloadPreview {
            kind: DownloadKind::Download,
            source_name,
            files: out,
            locations: locs,
            mode,
            current_files: disk.len(),
            existing: {
                let mut v: Vec<String> = known_set.into_iter().collect();
                v.sort();
                v
            },
        })
    }

    /// 导入下载的存档为一条新路线（接在当前快照之后）。不写入存档目录。
    pub fn import_download(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        password: Option<&str>,
        req: &DownloadImport,
    ) -> Result<DownloadReport> {
        if let Some(c) = &req.comment {
            crate::backup::check_comment(c)?;
        }
        let staging = self.download_staging("download");
        let result = self.import_download_in(lock, game, src, password, req, &staging);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn import_download_in(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        password: Option<&str>,
        req: &DownloadImport,
        staging: &Path,
    ) -> Result<DownloadReport> {
        let locations = self.locations(&game.id)?;
        let files = match gather(src, staging, password)? {
            Ok(f) => f,
            Err(DownloadKind::NeedsPassword) => {
                return Err(AppError::Invalid("这个压缩包需要解压密码".into()));
            }
            Err(DownloadKind::NeedsExtract) => {
                return Err(AppError::Invalid(
                    "请先用解压工具解压，再导入解压出来的文件夹".into(),
                ));
            }
            Err(_) => {
                return Err(AppError::Invalid(
                    "这是篝火导出的存档，请用“导入存档文件 / 篝火包”".into(),
                ));
            }
        };
        let by_rel: HashMap<&str, &PathBuf> = files.iter().map(|(r, a)| (r.as_str(), a)).collect();
        // 叠加时从当前磁盘上的存档开始。
        let mut chosen: BTreeMap<String, PathBuf> = BTreeMap::new();
        if req.mode == DownloadMode::Overlay {
            let (disk, _) = self.detect(game, false)?;
            for f in disk {
                chosen.insert(f.path.clone(), f.abs.clone());
            }
        }
        let mut imported = 0;
        for (rel, target) in &req.mapping {
            let abs = by_rel
                .get(rel.as_str())
                .ok_or_else(|| AppError::Invalid(format!("下载中没有文件：{rel}")))?;
            if !valid_target(target, locations.len()) {
                return Err(AppError::Invalid(format!("放置位置不正确：{target}")));
            }
            // 同一位置大小写不同的旧文件去掉。
            chosen.retain(|k, _| lower(k) != lower(target));
            chosen.insert(target.clone(), (*abs).clone());
            imported += 1;
        }
        if imported == 0 {
            return Err(AppError::Invalid("没有选择要导入的文件".into()));
        }
        // 内容与已有快照完全相同（按文件内容比较，不看修改时间）：不重复生成。
        let list: Vec<(String, PathBuf)> =
            chosen.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Some(id) = self.identical_snapshot(&game.id, &list)? {
            return Ok(DownloadReport {
                snapshot: None,
                identical: Some(id),
                branch: None,
                files: imported,
            });
        }
        let mut entries = Vec::with_capacity(chosen.len());
        for (path, abs) in &chosen {
            let f = std::fs::File::open(abs).map_err(|e| AppError::io(abs, e))?;
            let meta = f.metadata().map_err(|e| AppError::io(abs, e))?;
            let stats = self.repo.write_file(&f, abs)?;
            entries.push(TreeEntry {
                path: path.clone(),
                blob: stats.blob,
                size: stats.size,
                mtime_ns: scan::mtime_ns(&meta),
                tag: bonfire_core::object::FileTag::Save,
            });
        }
        let tree = Tree::new(entries);
        let tree_id = self.repo.put_tree(&tree)?;
        if let Some(id) = self.snapshot_with_tree(&game.id, &tree_id)? {
            return Ok(DownloadReport {
                snapshot: None,
                identical: Some(id),
                branch: None,
                files: imported,
            });
        }
        let source = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let comment = match req
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(c) => format!("下载的存档：{source}（{c}）"),
            None => format!("下载的存档：{source}"),
        };
        let parent = self.repo.head_snapshot(&game.id)?;
        let base = req
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("下载-{}", jiff::Zoned::now().strftime("%Y%m%d")));
        bonfire_core::refs::validate_ref_name(&base)?;
        let taken: HashSet<String> = self
            .repo
            .list_refs(&game.id)?
            .into_iter()
            .map(|(r, _)| lower(&r.name))
            .collect();
        let mut branch = base.clone();
        let mut i = 2;
        while taken.contains(&lower(&branch)) {
            branch = format!("{base}-{i}");
            i += 1;
        }
        let snap = Snapshot {
            game: game.id.clone(),
            tree: tree_id,
            parents: parent.into_iter().collect(),
            created_at: now_local_rfc3339(),
            trigger: Trigger::Import,
            host: host_name(),
            comment: comment.clone(),
            stats: SnapshotStats {
                files: tree.entries.len() as u64,
                bytes: tree.total_size(),
                changed: imported as u64,
            },
            playtime_min: None,
        };
        let id = self.repo.put_snapshot(&snap)?;
        self.repo.create_branch(lock, &game.id, &branch, id)?;
        self.index.add_snapshot(&id, &snap, &comment)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        tracing::info!(game = %game.id, %branch, files = imported, "导入下载的存档");
        Ok(DownloadReport {
            snapshot: Some(id),
            identical: None,
            branch: Some(branch),
            files: imported,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_map_finds_folders_by_file_names() {
        let known = vec![
            "0/user1.dat".to_string(),
            "0/user2.dat".to_string(),
            "0/shared.dat".to_string(),
            "1/settings.ini".to_string(),
        ];
        let files: Vec<(String, u64)> = [
            "全收集存档/存档/user1.dat",
            "全收集存档/存档/user3.dat",
            "全收集存档/存档/sub/extra.bin",
            "全收集存档/读我.txt",
            "全收集存档/settings.ini",
            "全收集存档/预览.png",
        ]
        .iter()
        .map(|s| (s.to_string(), 1))
        .collect();
        let t = auto_map(&files, &known, 2);
        assert_eq!(t[0].as_deref(), Some("0/user1.dat"));
        // 同一文件夹中没有同名文件的也跟着放进去。
        assert_eq!(t[1].as_deref(), Some("0/user3.dat"));
        assert_eq!(t[2].as_deref(), Some("0/sub/extra.bin"));
        assert_eq!(t[3], None);
        assert_eq!(t[4].as_deref(), Some("1/settings.ini"));
        assert_eq!(t[5], None);
        // 一个同名文件都没有、只有一个存档位置：去掉外层文件夹整个放进去。
        let t = auto_map(
            &[("pack/save/a.sav".into(), 1), ("pack/save/b.sav".into(), 1)],
            &[],
            1,
        );
        assert_eq!(t, vec![Some("0/a.sav".into()), Some("0/b.sav".into())]);
        assert!(
            valid_target("0/a/b.sav", 1) && !valid_target("1/a", 1) && !valid_target("0/../x", 1)
        );
    }

    #[test]
    fn import_encrypted_download_as_overlay_branch() {
        use crate::backup::BackupOptions;
        use crate::testutil::Env;
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("user1.dat", b"my slot 1");
        env.write("user2.dat", b"my slot 2");
        {
            let lock = env.app.repo.lock().unwrap();
            env.app
                .backup(&lock, &g, &BackupOptions::default())
                .unwrap();
        }
        let before = env.app.repo.head(&g.id).unwrap();

        // 加密的下载包：存档套在两层文件夹里，附带说明；里面还有一个 zip。
        let pkg = env.dir.path().join("全收集存档.zip");
        {
            let f = std::fs::File::create(&pkg).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .with_aes_encryption(zip::AesMode::Aes256, "pw123");
            zip.start_file("全收集存档/存档/user1.dat", opts).unwrap();
            zip.write_all(b"downloaded slot 1").unwrap();
            zip.start_file("全收集存档/使用说明.txt", opts).unwrap();
            zip.write_all(b"copy to save folder").unwrap();
            zip.finish().unwrap();
        }
        let p = env.app.inspect_download(&g, &pkg, None).unwrap();
        assert_eq!(p.kind, DownloadKind::NeedsPassword);
        assert!(env.app.inspect_download(&g, &pkg, Some("wrong")).is_err());
        let p = env.app.inspect_download(&g, &pkg, Some("pw123")).unwrap();
        assert_eq!(p.kind, DownloadKind::Download);
        let slot = p
            .files
            .iter()
            .find(|f| f.rel.ends_with("user1.dat"))
            .unwrap();
        assert_eq!(slot.target.as_deref(), Some("0/user1.dat"));
        assert!(slot.replaces);
        assert!(
            p.files
                .iter()
                .find(|f| f.rel.ends_with(".txt"))
                .unwrap()
                .ignored
        );
        // 只有部分文件：建议叠加。
        assert_eq!(p.mode, DownloadMode::Overlay);

        let lock = env.app.repo.lock().unwrap();
        let req = DownloadImport {
            mapping: vec![(slot.rel.clone(), "0/user1.dat".into())],
            mode: DownloadMode::Overlay,
            branch: None,
            comment: Some("B站 某某分享".into()),
        };
        let r = env
            .app
            .import_download(&lock, &g, &pkg, Some("pw123"), &req)
            .unwrap();
        let branch = r.branch.clone().unwrap();
        assert!(branch.starts_with("下载-"));
        // 新路线的快照：下载的 user1 + 原来的 user2（与配置文件），当前路线与磁盘不变。
        let tree = env
            .app
            .repo
            .get_tree(
                &env.app
                    .repo
                    .get_snapshot(&r.snapshot.unwrap())
                    .unwrap()
                    .tree,
            )
            .unwrap();
        let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"0/user1.dat") && paths.contains(&"0/user2.dat"),
            "{paths:?}"
        );
        assert_eq!(env.app.repo.head(&g.id).unwrap(), before);
        assert_eq!(env.read("user1.dat").unwrap(), b"my slot 1");
        let snap = env.app.repo.get_snapshot(&r.snapshot.unwrap()).unwrap();
        assert!(snap.comment.starts_with("下载的存档：全收集存档.zip"));
        // 再导入一次：内容相同，不重复生成。
        let again = env
            .app
            .import_download(&lock, &g, &pkg, Some("pw123"), &req)
            .unwrap();
        assert!(again.snapshot.is_none() && again.identical.is_some());
        // 7z 与篝火导出的存档：提示改用其他方式。
        let seven = env.dir.path().join("a.7z");
        std::fs::write(&seven, b"7z").unwrap();
        // 损坏的 7z：说明读取失败，可以先用解压工具解压。
        let err = env.app.inspect_download(&g, &seven, None).unwrap_err();
        assert!(err.to_string().contains("7z"), "{err}");
        let tgz = env.dir.path().join("a.tar.gz");
        std::fs::write(&tgz, b"gz").unwrap();
        assert_eq!(
            env.app.inspect_download(&g, &tgz, None).unwrap().kind,
            DownloadKind::NeedsExtract
        );
        let bf = env.dir.path().join("exported");
        std::fs::create_dir_all(&bf).unwrap();
        std::fs::write(bf.join(crate::transfer::EXPORT_MANIFEST), b"{}").unwrap();
        assert_eq!(
            env.app.inspect_download(&g, &bf, None).unwrap().kind,
            DownloadKind::Bonfire
        );
    }

    /// tests/inputs/downloads 中的压缩包（复制到临时目录再读，输入只读）。
    fn input_copy(env: &crate::testutil::Env, name: &str) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/inputs/downloads")
            .join(name);
        let dst = env.dir.path().join(name);
        std::fs::copy(&src, &dst).unwrap();
        dst
    }

    #[test]
    fn seven_zip_and_rar_are_unpacked_built_in() {
        use crate::testutil::Env;
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("slot1.sav", b"mine");

        // 7z（AES 加密，文件名也加密）：先要密码，密码不对报错，对了自动对应。
        let seven = input_copy(&env, "hk-7z-加密.7z");
        assert_eq!(
            env.app.inspect_download(&g, &seven, None).unwrap().kind,
            DownloadKind::NeedsPassword
        );
        let err = env
            .app
            .inspect_download(&g, &seven, Some("wrong"))
            .unwrap_err();
        assert!(err.to_string().contains("密码"), "{err}");
        let p = env.app.inspect_download(&g, &seven, Some("pw7z")).unwrap();
        assert_eq!(p.kind, DownloadKind::Download);
        let slot = p
            .files
            .iter()
            .find(|f| f.rel.ends_with("slot1.sav"))
            .unwrap();
        assert_eq!(slot.target.as_deref(), Some("0/slot1.sav"));
        assert!(p.files.iter().any(|f| f.rel.ends_with(".txt") && f.ignored));

        // rar（RAR 4，存储模式）：反斜杠路径、自动对应。
        let rar = input_copy(&env, "hk-rar.rar");
        let p = env.app.inspect_download(&g, &rar, None).unwrap();
        assert_eq!(p.kind, DownloadKind::Download, "{:?}", p.files);
        let slot = p
            .files
            .iter()
            .find(|f| f.rel.ends_with("slot1.sav"))
            .unwrap();
        assert_eq!(slot.rel, "pack/save/slot1.sav");
        assert_eq!(slot.target.as_deref(), Some("0/slot1.sav"));

        // 导入 rar 中的存档，内容来自压缩包。
        let lock = env.app.repo.lock().unwrap();
        let r = env
            .app
            .import_download(
                &lock,
                &g,
                &rar,
                None,
                &DownloadImport {
                    mapping: vec![(slot.rel.clone(), "0/slot1.sav".into())],
                    mode: DownloadMode::Replace,
                    branch: Some("rar".into()),
                    comment: None,
                },
            )
            .unwrap();
        let tree = env
            .app
            .repo
            .get_tree(
                &env.app
                    .repo
                    .get_snapshot(&r.snapshot.unwrap())
                    .unwrap()
                    .tree,
            )
            .unwrap();
        let e = tree
            .entries
            .iter()
            .find(|e| e.path == "0/slot1.sav")
            .unwrap();
        assert_eq!(e.size, b"level 88 from rar".len() as u64);
        assert!(safe_rel("../x").is_none() && safe_rel("C:\\x").is_none());
        assert_eq!(
            safe_rel("a\\b/c.sav"),
            Some(PathBuf::from("a").join("b").join("c.sav"))
        );
    }

    #[test]
    fn manual_replacement_is_flagged_and_backed_up_to_new_branch() {
        use crate::backup::BackupOptions;
        use crate::testutil::Env;
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("user1.dat", b"mine");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        assert!(!env.app.game_row(&g, false).unwrap().maybe_replaced);
        // 手动换成下载的存档：内容变了，修改时间是原作者电脑上的（早于上次备份）。
        env.write("user1.dat", b"downloaded");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400);
        std::fs::OpenOptions::new()
            .write(true)
            .open(env.saves().join("user1.dat"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let row = env.app.game_row(&g, true).unwrap();
        assert!(row.maybe_replaced);
        // 另开新路线备份：原来的 main 不动。
        let main_tip = env
            .app
            .repo
            .read_ref(&g.id, &bonfire_core::refs::RefName::branch("main"))
            .unwrap();
        let r = env
            .app
            .backup(
                &lock,
                &g,
                &BackupOptions {
                    new_branch: Some("下载-测试".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(r, crate::backup::BackupResult::Created { .. }));
        assert_eq!(env.app.repo.head(&g.id).unwrap().branch, "下载-测试");
        assert_eq!(
            env.app
                .repo
                .read_ref(&g.id, &bonfire_core::refs::RefName::branch("main"))
                .unwrap(),
            main_tip
        );
        assert!(!env.app.game_row(&g, false).unwrap().maybe_replaced);

        // 新出现的旧文件（如存档位置变化、新的存档槽）不算手动替换。
        env.write("slot9.dat", b"old file appears");
        std::fs::OpenOptions::new()
            .write(true)
            .open(env.saves().join("slot9.dat"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!env.app.game_row(&g, true).unwrap().maybe_replaced);
        // 游戏正常写存档（时间往后走）不算。
        env.write("user1.dat", b"played more");
        assert!(!env.app.game_row(&g, true).unwrap().maybe_replaced);
    }
}
