//! 导出与导入（文档 F10），用于迁移、分享存档。
//!
//! - **导出存档文件**：当前磁盘上的存档或某个快照，打包为 zip 或复制到文件夹，每个存档位置一个子文件夹，
//!   附带 `bonfire-export.json` 说明来源。拿到的人可以直接使用，也可以用“导入存档文件”收进自己的历史。
//! - **导出历史包**（`.bonfire`）：全部分支、选定分支或一段历史中的快照对象与分支指针。
//! - **导入历史包**：对象按内容寻址去重；快照 ID 由内容与父快照决定，所以用 ID 判断导入的分支与本地同名分支的关系：
//!   相同、后续更新（快进）、本地已包含、分叉（另建分支，不覆盖本地）。
//! - **导入存档文件**：生成一个新快照（触发方式“导入”），放在新分支上或接在某个分支之后；内容与已有快照相同时不重复生成。
//!
//! - **合集包**（也是 `.bonfire`）：多个游戏的历史包放在一个文件里（`collection.json` + `games/<序号>/…`），
//!   用于一次导出、导入全部或选定的游戏。
//! - **统一的篝火包**（U-16）：游戏库、详情页与图库的导出都用合集包格式，一个或多个游戏，每个游戏可以只有存档、
//!   只有截图或两者都有（`bundle.json` 中没有快照时只含截图）。导入时按游戏选择导入存档、截图或两者；
//!   截图合并到相册（已有的跳过），可选把新导入的截图写到本机这个游戏的截图文件夹。
//! - 导入时游戏库中没有的游戏（如这台电脑上没有安装）自动加入游戏库，标为“未安装”：
//!   能在本机确定的存档位置照常使用，需要安装目录的位置等装好游戏、扫描后再确定。
//!
//! 导入只写入仓库，不写入存档目录；要用导入的存档玩，在历史中选中后正常恢复（恢复前会自动建保护快照）。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bonfire_core::index::{GameRecord, LocationKind, LocationRecord, ShotRecord};
use bonfire_core::object::{FileTag, Kind, Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
use bonfire_core::refs::{RefKind, RefName};
use bonfire_core::scan::{self, glob_base};
use bonfire_core::{ObjectId, RepoLock};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::error::{AppError, Result};
use crate::util::{host_name, now_compact, now_local_rfc3339};

/// 导出的存档文件中说明来源的文件名。
pub const EXPORT_MANIFEST: &str = "bonfire-export.json";
/// 历史包中的说明文件名。
const BUNDLE_MANIFEST: &str = "bundle.json";
const BUNDLE_FORMAT: &str = "bonfire-bundle";
const FILES_FORMAT: &str = "bonfire-files";
/// 历史包的扩展名。
pub const BUNDLE_EXT: &str = "bonfire";
/// 合集包的说明文件名。
const COLLECTION_MANIFEST: &str = "collection.json";
const COLLECTION_FORMAT: &str = "bonfire-collection";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Zip,
    Folder,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// 生成的 zip 文件或文件夹。
    pub dest: PathBuf,
    pub files: u64,
    pub bytes: u64,
    /// 导出的快照；导出当前存档时为空。
    pub snapshot: Option<ObjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportLocation {
    pub ord: usize,
    pub folder: String,
    pub path_template: String,
    pub resolved_path: String,
    pub kind: LocationKind,
}

/// `bonfire-export.json`：导出的存档来自哪个游戏、哪个快照，每个子文件夹对应哪个存档位置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    pub format: String,
    pub version: u32,
    /// 生成者，如 `Bonfire 0.1.0`（U-47，导入时识别；早期导出的没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub game_id: String,
    pub game_name: String,
    #[serde(default)]
    pub snapshot: Option<ObjectId>,
    #[serde(default)]
    pub snapshot_created_at: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    pub exported_at: String,
    pub host: String,
    pub locations: Vec<ExportLocation>,
}

/// 历史包的导出范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum BundleScope {
    /// 全部分支与标签。
    All,
    /// 选定的分支（含其全部历史）。
    Branches { names: Vec<String> },
    /// 一段历史：从 `from`（含；省略表示从头开始）到 `to`（含）。
    Range { from: Option<String>, to: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRef {
    pub kind: RefKind,
    pub name: String,
    pub target: ObjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleLocation {
    pub path_template: String,
    pub kind: LocationKind,
    /// 存档或配置文件；旧版历史包没有，按存档处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<FileTag>,
}

/// `bundle.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    pub format: String,
    pub version: u32,
    /// 生成者，如 `Bonfire 0.1.0`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub game_id: String,
    pub game_name: String,
    pub created_at: String,
    pub host: String,
    pub refs: Vec<BundleRef>,
    /// 包内的快照，父快照在前。
    pub snapshots: Vec<ObjectId>,
    /// 包内快照的父快照中不在包内的：导入方必须已有这些快照（只导出了一段历史时）。
    pub prerequisites: Vec<ObjectId>,
    /// 事后修改过的备注。
    #[serde(default)]
    pub notes: BTreeMap<ObjectId, String>,
    pub locations: Vec<BundleLocation>,
    /// 一并导出的截图相册（导出时勾选“包含截图”才有）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shots: Vec<ShotRecord>,
    /// 导出方这个游戏只管理截图：导入为新游戏时同样设为只管理截图。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shots_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleReport {
    pub dest: PathBuf,
    pub snapshots: usize,
    pub objects: usize,
    /// 历史包文件大小。
    pub bytes: u64,
    pub refs: Vec<BundleRef>,
    pub prerequisites: usize,
    /// 一并导出的截图数。
    pub shots: usize,
}

/// 导入历史包时，包内一个分支或标签与本地的关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefAction {
    /// 本地没有同名分支（标签）：新建。
    Create,
    /// 与本地同名分支指向同一个快照。
    Same,
    /// 包内的是本地分支的后续更新：分支前移（快进）。
    FastForward,
    /// 本地分支已包含包内的快照（本地更新）：不改动。
    Contained,
    /// 双方各自有新快照：另建分支，不覆盖本地。
    Diverged,
    /// 同名标签指向别的快照：跳过。
    TagConflict,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefPlan {
    pub kind: RefKind,
    pub name: String,
    pub target: ObjectId,
    pub action: RefAction,
    /// 本地同名引用指向的快照。
    pub local_target: Option<ObjectId>,
    /// 分叉时新建的分支名。
    pub new_name: Option<String>,
    /// 分叉点：双方共同的最近快照。
    pub fork_point: Option<ObjectId>,
}

/// 包内的一个快照，供界面画分支图。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleNode {
    pub id: ObjectId,
    pub parents: Vec<ObjectId>,
    pub created_at: String,
    pub trigger: Trigger,
    pub comment: String,
    pub files: u64,
    pub bytes: u64,
    pub host: String,
    /// 本地已有这个快照。
    pub known: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub game_id: String,
    pub game_name: String,
    /// 本地游戏库中有这个游戏。
    pub local_game: bool,
    /// 导入时会把这个游戏加入游戏库（本地没有这个游戏，标为“未安装”）。
    pub create_game: bool,
    pub created_at: String,
    pub host: String,
    pub nodes: Vec<BundleNode>,
    pub new_snapshots: usize,
    /// 本地缺少的前置快照：需要先导入更早的一段历史。
    pub missing: Vec<ObjectId>,
    pub refs: Vec<RefPlan>,
    /// 包内的存档位置与本地不同（来自另一种安装方式或另一台电脑的自定义路径）。
    pub location_mismatch: bool,
    /// 包内的截图数，其中本地相册没有的。
    pub shots: usize,
    pub new_shots: usize,
    /// 包内的 main 与本机的 main 分叉，导入时改成的名字（如 main-旧电脑）。导入页据此询问是否以它为本机主存档。
    pub main_conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleImportReport {
    pub preview: BundlePreview,
    pub imported_snapshots: usize,
    pub objects: usize,
    /// 新加入游戏库的游戏（本地原来没有）。
    pub created_game: bool,
    /// 加入相册的截图数。
    pub imported_shots: usize,
    /// 加入相册的截图（用于把新导入的截图写到磁盘）。
    #[serde(default)]
    pub new_shot_ids: Vec<ObjectId>,
}

/// 合集包中的一个游戏。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEntry {
    pub game_id: String,
    pub game_name: String,
    /// 包内的目录，如 `games/0/`。
    pub dir: String,
    pub snapshots: usize,
    /// 包内的截图数（早期的包没有这一项）。
    #[serde(default)]
    pub shots: usize,
}

/// `collection.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionManifest {
    pub format: String,
    pub version: u32,
    /// 生成者，如 `Bonfire 0.1.0`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub created_at: String,
    pub host: String,
    pub games: Vec<CollectionEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionReport {
    pub dest: PathBuf,
    pub bytes: u64,
    pub games: Vec<CollectionEntry>,
    pub objects: usize,
    /// 还没有快照、没有导出的游戏（名称）。
    pub skipped: Vec<String>,
}

/// 历史包或合集包中的一个游戏的预览。`key` 是包内目录（单个游戏的历史包为空）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveGame {
    pub key: String,
    pub preview: BundlePreview,
}

/// 历史包或合集包的预览。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePreview {
    pub collection: bool,
    pub created_at: String,
    pub host: String,
    pub games: Vec<ArchiveGame>,
}

/// 导入合集包时选中的游戏：`key` 为包内目录，`target` 为导入到的本地游戏（省略时按游戏 ID 对应或新建）；
/// `saves` / `shots` 选择导入存档历史、截图（默认都导入）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePick {
    pub key: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default = "yes")]
    pub saves: bool,
    #[serde(default = "yes")]
    pub shots: bool,
    /// 这是我旧电脑的存档：包内的 main 与本机分叉时，以它为本机主存档并整理名称（U-41h，必须手动选）。
    #[serde(default)]
    pub adopt_main: bool,
}

fn yes() -> bool {
    true
}

impl ArchivePick {
    /// 导入这个游戏的全部内容。
    pub fn all(key: impl Into<String>) -> Self {
        ArchivePick {
            key: key.into(),
            target: None,
            saves: true,
            shots: true,
            adopt_main: false,
        }
    }
}

/// 导出篝火包的内容（U-16）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageOptions {
    /// 导出存档历史（全部分支与标签）。
    pub saves: bool,
    /// 导出截图相册（不含回收站）。
    pub shots: bool,
    /// 只导出这些截图（按游戏）；没有列出的游戏导出全部截图。
    #[serde(default)]
    pub shot_ids: HashMap<String, Vec<ObjectId>>,
}

/// 历史包中包含哪些截图。
enum ShotSel<'a> {
    None,
    All,
    Only(&'a HashSet<ObjectId>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveImportItem {
    pub key: String,
    pub game_id: String,
    pub game_name: String,
    pub report: Option<BundleImportReport>,
    /// 这个游戏导入失败的原因（其他游戏照常导入）。
    pub error: Option<String>,
    /// 以导入的路线为本机主存档后它的名字（界面随后恢复它的最新快照）。
    #[serde(default)]
    pub adopted_main: Option<String>,
}

/// 导入的存档文件放在哪里。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImportPlace {
    /// 新建分支；`parent` 为空时是一条独立的新起点。
    NewBranch {
        name: Option<String>,
        parent: Option<String>,
    },
    /// 接在某个分支的最新快照之后，分支前移。
    Continue { branch: String },
}

impl Default for ImportPlace {
    fn default() -> Self {
        ImportPlace::NewBranch {
            name: None,
            parent: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportFilesOptions {
    pub place: ImportPlace,
    pub comment: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMap {
    /// 来源中的顶层文件夹（空表示根目录）。
    pub folder: String,
    /// 对应的本地存档位置序号；`None` 表示无法对应。
    pub ord: Option<usize>,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPreview {
    pub files: u64,
    pub bytes: u64,
    pub folders: Vec<FolderMap>,
    /// 无法对应到存档位置的文件。
    pub unmapped: Vec<String>,
    /// 来源附带的说明（由篝火导出时才有）。
    pub manifest: Option<ExportManifest>,
    /// 内容与本地某个快照完全相同。
    pub identical: Option<ObjectId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFilesReport {
    pub preview: FilesPreview,
    /// 新生成的快照；内容与已有快照相同或只是预览时为空。
    pub snapshot: Option<ObjectId>,
    pub branch: Option<String>,
    pub dry_run: bool,
}

/// 一个历史包的内容：说明与要打包的对象（快照即说明中的 `snapshots`）。
struct BundlePlan {
    manifest: BundleManifest,
    chunks: Vec<ObjectId>,
    blobs: Vec<ObjectId>,
    trees: Vec<ObjectId>,
    /// 要放入包中的截图（本地记录，原图从这里读取）。
    shots: Vec<ShotRecord>,
}

impl BundlePlan {
    fn objects(&self) -> usize {
        self.chunks.len() + self.blobs.len() + self.trees.len() + self.manifest.snapshots.len()
    }
}

/// 在游戏库中导入存档文件：按附带的说明找到（或新建）游戏。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesSource {
    pub manifest: ExportManifest,
    /// 游戏库中对应的游戏；没有时导入会新建（未安装）。
    pub game_id: Option<String>,
    /// 已有游戏时的预览。
    pub preview: Option<FilesPreview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesAutoReport {
    pub game_id: String,
    pub game_name: String,
    pub created_game: bool,
    pub report: ImportFilesReport,
}

/// 来源中的文件：(相对路径, 磁盘文件)。
type SourceFiles = Vec<(String, PathBuf)>;
/// 待导出的文件：(清单路径, 大小, 内容来源)。
type ExportItems = Vec<(String, u64, Source)>;

/// 待导出的一个文件的内容来源。
enum Source {
    Blob(ObjectId),
    Disk(PathBuf),
}

impl App {
    /// 导出文件的默认名称（不含扩展名），如 `Hollow Knight-20261009-211403-1a2b3c4d`。
    pub fn export_default_name(&self, game: &GameRecord, spec: Option<&str>) -> Result<String> {
        let label = match spec {
            Some(s) => {
                let id = self.repo.resolve(&game.id, s)?;
                let snap = self.repo.get_snapshot(&id)?;
                let time: String = snap
                    .created_at
                    .chars()
                    .take(19)
                    .filter(|c| c.is_ascii_digit() || *c == 'T')
                    .collect();
                format!("{}-{}", time.replace('T', "-"), &id.to_hex()[..8])
            }
            None => now_compact(),
        };
        Ok(format!(
            "{}{}-{label}",
            crate::branding::FILE_PREFIX,
            sanitize(&game.name)
        ))
    }

    /// 导出存档文件：`spec` 为快照（分支、标签、ID）时导出该快照，为空时导出当前磁盘上的存档。
    /// zip：`dest` 是要生成的文件；文件夹：`dest` 是上级目录，在其中新建以默认名称命名的文件夹。
    pub fn export_files(
        &self,
        game: &GameRecord,
        spec: Option<&str>,
        dest: &Path,
        format: ExportFormat,
    ) -> Result<ExportReport> {
        let locations = self.locations(&game.id)?;
        let folders = location_folders(&locations);
        let (items, snapshot): (ExportItems, Option<(ObjectId, Snapshot)>) = match spec {
            Some(s) => {
                let id = self.repo.resolve(&game.id, s)?;
                let snap = self.repo.get_snapshot(&id)?;
                let tree = self.repo.get_tree(&snap.tree)?;
                (
                    tree.entries
                        .into_iter()
                        .map(|e| (e.path, e.size, Source::Blob(e.blob)))
                        .collect(),
                    Some((id, snap)),
                )
            }
            None => (
                scan::collect_files(&self.scan_roots(game)?)?
                    .into_iter()
                    .map(|f| (f.path, f.size, Source::Disk(f.abs)))
                    .collect(),
                None,
            ),
        };
        if items.is_empty() {
            return Err(AppError::Invalid(format!(
                "游戏「{}」没有可导出的存档文件",
                game.name
            )));
        }
        let manifest = ExportManifest {
            format: FILES_FORMAT.into(),
            version: 1,
            generator: Some(crate::branding::generator()),
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            snapshot: snapshot.as_ref().map(|(id, _)| *id),
            snapshot_created_at: snapshot.as_ref().map(|(_, s)| s.created_at.clone()),
            comment: match &snapshot {
                Some((id, s)) => Some(self.repo.effective_comment(id, s)?),
                None => None,
            },
            exported_at: now_local_rfc3339(),
            host: host_name(),
            locations: locations
                .iter()
                .enumerate()
                .map(|(ord, l)| ExportLocation {
                    ord,
                    folder: folders[ord].clone(),
                    path_template: l.path_template.clone(),
                    resolved_path: l.resolved_path.clone(),
                    kind: l.kind,
                })
                .collect(),
        };
        let out_name = |tree_path: &str| -> String {
            let (ord, rel) = tree_path.split_once('/').unwrap_or(("0", tree_path));
            let folder = ord
                .parse::<usize>()
                .ok()
                .and_then(|i| folders.get(i))
                .cloned()
                .unwrap_or_else(|| format!("location-{ord}"));
            format!("{folder}/{rel}")
        };
        let target = match format {
            ExportFormat::Zip => dest.to_path_buf(),
            ExportFormat::Folder => unique_child(dest, &self.export_default_name(game, spec)?),
        };
        self.check_export_target(&target)?;

        let mut bytes = 0u64;
        let write_item = |src: &Source, out: &mut dyn Write, label: &Path| -> Result<u64> {
            match src {
                Source::Blob(b) => Ok(self.repo.read_blob_to(b, out, label)?),
                Source::Disk(p) => {
                    let mut f = std::fs::File::open(p).map_err(|e| AppError::io(p, e))?;
                    std::io::copy(&mut f, out).map_err(|e| AppError::io(p, e))
                }
            }
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest).expect("说明文件可以序列化");
        let readme = crate::branding::readme(
            &crate::branding::ReadmeKind::Files {
                game: &manifest.game_name,
                snapshot: manifest.snapshot.as_ref().map(|s| s.short(12)).as_deref(),
                comment: manifest.comment.as_deref(),
                locations: manifest
                    .locations
                    .iter()
                    .map(|l| (l.folder.clone(), l.path_template.clone()))
                    .collect(),
            },
            &manifest.exported_at,
        );
        match format {
            ExportFormat::Zip => {
                let tmp = with_suffix(&target, ".bonfire-tmp");
                let result = (|| -> Result<()> {
                    let file = std::fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
                    let mut zip = zip::ZipWriter::new(file);
                    let when = zip_time(snapshot.as_ref().map(|(_, s)| s.created_at.as_str()));
                    for (path, size, src) in &items {
                        let opts = zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated)
                            .large_file(*size >= u32::MAX as u64)
                            .last_modified_time(when);
                        zip.start_file(out_name(path), opts)
                            .map_err(|e| zip_err(&tmp, e))?;
                        bytes += write_item(src, &mut zip, &tmp)?;
                    }
                    zip.start_file(
                        EXPORT_MANIFEST,
                        zip::write::SimpleFileOptions::default().last_modified_time(when),
                    )
                    .map_err(|e| zip_err(&tmp, e))?;
                    zip.write_all(&manifest_json)
                        .map_err(|e| AppError::io(&tmp, e))?;
                    zip.start_file(
                        crate::branding::README_NAME,
                        zip::write::SimpleFileOptions::default().last_modified_time(when),
                    )
                    .map_err(|e| zip_err(&tmp, e))?;
                    zip.write_all(&readme).map_err(|e| AppError::io(&tmp, e))?;
                    let f = zip.finish().map_err(|e| zip_err(&tmp, e))?;
                    f.sync_all().map_err(|e| AppError::io(&tmp, e))?;
                    Ok(())
                })();
                if let Err(e) = result {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                replace_file(&tmp, &target)?;
            }
            ExportFormat::Folder => {
                let tmp = with_suffix(&target, ".bonfire-tmp");
                let result = (|| -> Result<()> {
                    for (path, _, src) in &items {
                        let out = safe_join(&tmp, &out_name(path))?;
                        if let Some(dir) = out.parent() {
                            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
                        }
                        let mut f =
                            std::fs::File::create(&out).map_err(|e| AppError::io(&out, e))?;
                        bytes += write_item(src, &mut f, &out)?;
                    }
                    let m = tmp.join(EXPORT_MANIFEST);
                    std::fs::write(&m, &manifest_json).map_err(|e| AppError::io(&m, e))?;
                    let r = tmp.join(crate::branding::README_NAME);
                    std::fs::write(&r, &readme).map_err(|e| AppError::io(&r, e))
                })();
                if let Err(e) = result {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return Err(e);
                }
                std::fs::rename(&tmp, &target).map_err(|e| AppError::io(&target, e))?;
            }
        }
        Ok(ExportReport {
            dest: target,
            files: items.len() as u64,
            bytes,
            snapshot: snapshot.map(|(id, _)| id),
        })
    }

    /// 导出目标不能在任何游戏的存档目录或仓库中（导出绝不写入存档目录）。
    pub(crate) fn check_export_target(&self, target: &Path) -> Result<()> {
        let t = path_key(target);
        let inside = |base: &Path| {
            let b = path_key(base);
            !b.is_empty() && (t == b || t.starts_with(&format!("{b}/")))
        };
        if inside(self.repo.root()) {
            return Err(AppError::Invalid(format!(
                "不能导出到仓库目录中：{}",
                target.display()
            )));
        }
        for g in self.all_games()? {
            for l in self.locations(&g.id)? {
                let base = match l.kind {
                    LocationKind::Dir => PathBuf::from(&l.resolved_path),
                    LocationKind::Glob => glob_base(&l.resolved_path),
                    LocationKind::File => PathBuf::from(&l.resolved_path),
                    LocationKind::Registry => continue,
                };
                if inside(&base) {
                    return Err(AppError::Invalid(format!(
                        "不能导出到游戏「{}」的存档位置中：{}",
                        g.name,
                        target.display()
                    )));
                }
            }
        }
        Ok(())
    }

    // ---------- 历史包 ----------

    /// 导出历史包；`include_shots` 时一并导出截图相册。
    pub fn export_bundle(
        &self,
        game: &GameRecord,
        scope: &BundleScope,
        dest: &Path,
        include_shots: bool,
    ) -> Result<BundleReport> {
        let shots = if include_shots {
            ShotSel::All
        } else {
            ShotSel::None
        };
        let plan = self.plan_bundle(game, Some(scope), shots)?;
        self.check_export_target(dest)?;
        let tmp = with_suffix(dest, ".bonfire-tmp");
        let result = (|| -> Result<()> {
            let file = std::fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
            let mut zip = zip::ZipWriter::new(file);
            self.write_bundle(&mut zip, &plan, "", &tmp)?;
            zip.start_file(
                crate::branding::README_NAME,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(|e| zip_err(&tmp, e))?;
            zip.write_all(&crate::branding::readme(
                &crate::branding::ReadmeKind::Package {
                    games: vec![(
                        plan.manifest.game_name.clone(),
                        plan.manifest.snapshots.len(),
                        plan.manifest.shots.len(),
                    )],
                },
                &plan.manifest.created_at,
            ))
            .map_err(|e| AppError::io(&tmp, e))?;
            let f = zip.finish().map_err(|e| zip_err(&tmp, e))?;
            f.sync_all().map_err(|e| AppError::io(&tmp, e))
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        replace_file(&tmp, dest)?;
        let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        Ok(BundleReport {
            dest: dest.to_path_buf(),
            snapshots: plan.manifest.snapshots.len(),
            objects: plan.objects(),
            bytes,
            refs: plan.manifest.refs.clone(),
            prerequisites: plan.manifest.prerequisites.len(),
            shots: plan.manifest.shots.len(),
        })
    }

    /// 导出多个游戏的全部历史到一个合集包。还没有快照的游戏跳过（勾选截图时只有截图的游戏也导出）。
    pub fn export_collection(
        &self,
        games: &[GameRecord],
        dest: &Path,
        include_shots: bool,
    ) -> Result<CollectionReport> {
        self.export_package(
            games,
            dest,
            &PackageOptions {
                saves: true,
                shots: include_shots,
                shot_ids: HashMap::new(),
            },
        )
    }

    /// 导出篝火包（U-16）：一个或多个游戏的存档历史和 / 或截图。没有可导出内容的游戏跳过。
    pub fn export_package(
        &self,
        games: &[GameRecord],
        dest: &Path,
        opts: &PackageOptions,
    ) -> Result<CollectionReport> {
        if games.is_empty() {
            return Err(AppError::Invalid("请至少选择一个游戏".into()));
        }
        if !opts.saves && !opts.shots {
            return Err(AppError::Invalid("请至少选择导出存档或截图".into()));
        }
        let filters: HashMap<&str, HashSet<ObjectId>> = opts
            .shot_ids
            .iter()
            .map(|(g, ids)| (g.as_str(), ids.iter().copied().collect()))
            .collect();
        let mut plans = Vec::new();
        let mut skipped = Vec::new();
        for g in games {
            let saves = opts.saves && !self.repo.list_refs(&g.id)?.is_empty();
            let shots = match (opts.shots, filters.get(g.id.as_str())) {
                (false, _) => ShotSel::None,
                (true, Some(only)) => ShotSel::Only(only),
                (true, None) => ShotSel::All,
            };
            let plan = self.plan_bundle(g, saves.then_some(&BundleScope::All), shots)?;
            if plan.manifest.snapshots.is_empty() && plan.manifest.shots.is_empty() {
                skipped.push(g.name.clone());
                continue;
            }
            plans.push(plan);
        }
        if plans.is_empty() {
            return Err(AppError::Invalid(
                if opts.saves && !opts.shots {
                    "选中的游戏都还没有快照"
                } else {
                    "选中的游戏没有可导出的存档或截图"
                }
                .into(),
            ));
        }
        let entries: Vec<CollectionEntry> = plans
            .iter()
            .enumerate()
            .map(|(i, p)| CollectionEntry {
                game_id: p.manifest.game_id.clone(),
                game_name: p.manifest.game_name.clone(),
                dir: format!("games/{i}/"),
                snapshots: p.manifest.snapshots.len(),
                shots: p.manifest.shots.len(),
            })
            .collect();
        let manifest = CollectionManifest {
            format: COLLECTION_FORMAT.into(),
            version: 1,
            generator: Some(crate::branding::generator()),
            created_at: now_local_rfc3339(),
            host: host_name(),
            games: entries.clone(),
        };
        self.check_export_target(dest)?;
        let tmp = with_suffix(dest, ".bonfire-tmp");
        let result = (|| -> Result<()> {
            let file = std::fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
            let mut zip = zip::ZipWriter::new(file);
            let deflate = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file(COLLECTION_MANIFEST, deflate)
                .map_err(|e| zip_err(&tmp, e))?;
            zip.write_all(&serde_json::to_vec_pretty(&manifest).expect("说明文件可以序列化"))
                .map_err(|e| AppError::io(&tmp, e))?;
            zip.start_file(crate::branding::README_NAME, deflate)
                .map_err(|e| zip_err(&tmp, e))?;
            zip.write_all(&crate::branding::readme(
                &crate::branding::ReadmeKind::Package {
                    games: entries
                        .iter()
                        .map(|e| (e.game_name.clone(), e.snapshots, e.shots))
                        .collect(),
                },
                &manifest.created_at,
            ))
            .map_err(|e| AppError::io(&tmp, e))?;
            for (plan, e) in plans.iter().zip(&entries) {
                self.write_bundle(&mut zip, plan, &e.dir, &tmp)?;
            }
            let f = zip.finish().map_err(|e| zip_err(&tmp, e))?;
            f.sync_all().map_err(|e| AppError::io(&tmp, e))
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        replace_file(&tmp, dest)?;
        Ok(CollectionReport {
            dest: dest.to_path_buf(),
            bytes: std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0),
            games: entries,
            objects: plans.iter().map(BundlePlan::objects).sum(),
            skipped,
        })
    }

    /// 历史包的内容：说明与要打包的对象。`scope` 为空时不含存档历史（只导出截图）。
    fn plan_bundle(
        &self,
        game: &GameRecord,
        scope: Option<&BundleScope>,
        shot_sel: ShotSel,
    ) -> Result<BundlePlan> {
        let all_refs = self.repo.list_refs(&game.id)?;
        let (included, mut refs) = match scope {
            None => (HashSet::new(), Vec::new()),
            Some(scope) => match scope {
                BundleScope::All => {
                    let tips: Vec<ObjectId> = all_refs.iter().map(|(_, id)| *id).collect();
                    let refs = all_refs
                        .iter()
                        .filter(|(r, _)| r.kind == RefKind::Branch)
                        .map(|(r, id)| BundleRef {
                            kind: r.kind,
                            name: r.name.clone(),
                            target: *id,
                        })
                        .collect();
                    (self.ancestors(&tips, &HashMap::new()), refs)
                }
                BundleScope::Branches { names } => {
                    if names.is_empty() {
                        return Err(AppError::Invalid("请至少选择一个分支".into()));
                    }
                    let mut refs = Vec::new();
                    for n in names {
                        let id = self
                            .repo
                            .read_ref(&game.id, &RefName::branch(n))?
                            .ok_or_else(|| AppError::Invalid(format!("找不到分支「{n}」")))?;
                        refs.push(BundleRef {
                            kind: RefKind::Branch,
                            name: n.clone(),
                            target: id,
                        });
                    }
                    let tips: Vec<ObjectId> = refs.iter().map(|r| r.target).collect();
                    (self.ancestors(&tips, &HashMap::new()), refs)
                }
                BundleScope::Range { from, to } => {
                    let to_id = self.repo.resolve(&game.id, to)?;
                    let mut set = self.ancestors(&[to_id], &HashMap::new());
                    if let Some(from) = from {
                        let from_id = self.repo.resolve(&game.id, from)?;
                        if !set.contains(&from_id) {
                            return Err(AppError::Invalid(format!(
                                "起点 {from} 不在终点 {to} 之前的历史中"
                            )));
                        }
                        let parents = self.repo.get_snapshot(&from_id)?.parents;
                        let before = self.ancestors(&parents, &HashMap::new());
                        set.retain(|id| !before.contains(id));
                    }
                    // 段的终点所在的分支：终点是分支最新快照时用该分支，否则用包含终点的分支名（导入方可快进到终点）。
                    let mut name = all_refs
                        .iter()
                        .find(|(r, id)| r.kind == RefKind::Branch && *id == to_id)
                        .map(|(r, _)| r.name.clone());
                    if name.is_none() {
                        let head = self.repo.head(&game.id)?;
                        let mut branches: Vec<&(RefName, ObjectId)> = all_refs
                            .iter()
                            .filter(|(r, _)| r.kind == RefKind::Branch)
                            .collect();
                        branches.sort_by_key(|(r, _)| r.name != head.branch);
                        name = branches
                            .into_iter()
                            .find(|(_, tip)| {
                                self.ancestors(&[*tip], &HashMap::new()).contains(&to_id)
                            })
                            .map(|(r, _)| r.name.clone());
                    }
                    let name = name.unwrap_or_else(|| format!("segment-{}", &to_id.to_hex()[..8]));
                    (
                        set,
                        vec![BundleRef {
                            kind: RefKind::Branch,
                            name,
                            target: to_id,
                        }],
                    )
                }
            },
        };
        if scope.is_some() && included.is_empty() {
            return Err(AppError::Invalid(format!(
                "游戏「{}」还没有快照",
                game.name
            )));
        }
        // 指向包内快照的标签一并导出。
        for (r, id) in &all_refs {
            if r.kind == RefKind::Tag && included.contains(id) {
                refs.push(BundleRef {
                    kind: RefKind::Tag,
                    name: r.name.clone(),
                    target: *id,
                });
            }
        }

        // 包内快照按父在前排序；收集前置快照与需要的对象。
        let mut snaps: HashMap<ObjectId, Snapshot> = HashMap::new();
        for id in &included {
            snaps.insert(*id, self.repo.get_snapshot(id)?);
        }
        let order = topo_order(&snaps);
        let mut prerequisites: Vec<ObjectId> = snaps
            .values()
            .flat_map(|s| s.parents.iter().copied())
            .filter(|p| !included.contains(p))
            .collect();
        prerequisites.sort();
        prerequisites.dedup();
        // 导入方已有前置快照的文件，不必重复打包。
        let mut known_blobs = HashSet::new();
        for p in &prerequisites {
            for e in self
                .repo
                .get_tree(&self.repo.get_snapshot(p)?.tree)?
                .entries
            {
                known_blobs.insert(e.blob);
            }
        }
        let mut trees = Vec::new();
        let mut blobs = Vec::new();
        let mut seen = HashSet::new();
        for id in &order {
            let tree_id = snaps[id].tree;
            if !seen.insert((Kind::Tree, tree_id)) {
                continue;
            }
            trees.push(tree_id);
            for e in self.repo.get_tree(&tree_id)?.entries {
                if !known_blobs.contains(&e.blob) && seen.insert((Kind::Blob, e.blob)) {
                    blobs.push(e.blob);
                }
            }
        }
        // 截图：相册中的（不含回收站），原图原样放在包中的 `shots/` 下。
        let shots: Vec<ShotRecord> = match shot_sel {
            ShotSel::None => Vec::new(),
            ShotSel::All | ShotSel::Only(_) => self
                .index
                .shots_of(&game.id)?
                .into_iter()
                .filter(|s| !s.purged && s.deleted_at.is_none())
                .filter(|s| match &shot_sel {
                    ShotSel::Only(ids) => ids.contains(&s.blob),
                    _ => true,
                })
                .collect(),
        };
        let mut chunks = Vec::new();
        for b in &blobs {
            for c in self.repo.get_blob(b)?.chunks {
                if seen.insert((Kind::Chunk, c)) {
                    chunks.push(c);
                }
            }
        }
        let mut notes = BTreeMap::new();
        for id in &order {
            if let Some(n) = self.repo.note(id)? {
                notes.insert(*id, n.comment);
            }
        }
        let manifest = BundleManifest {
            format: BUNDLE_FORMAT.into(),
            version: 1,
            generator: Some(crate::branding::generator()),
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            created_at: now_local_rfc3339(),
            host: host_name(),
            refs: refs.clone(),
            snapshots: order.clone(),
            prerequisites: prerequisites.clone(),
            notes,
            locations: self
                .locations(&game.id)?
                .iter()
                .map(|l| BundleLocation {
                    path_template: l.path_template.clone(),
                    kind: l.kind,
                    tag: Some(l.tag),
                })
                .collect(),
            shots: shots
                .iter()
                .map(|s| ShotRecord {
                    file: String::new(),
                    ..s.clone()
                })
                .collect(),
            shots_only: self.is_shots_only(&game.id)?,
        };
        Ok(BundlePlan {
            manifest,
            chunks,
            blobs,
            trees,
            shots,
        })
    }

    /// 把一个游戏的历史包写入压缩包中的 `prefix` 目录（单个历史包为空）。
    fn write_bundle(
        &self,
        zip: &mut zip::ZipWriter<std::fs::File>,
        plan: &BundlePlan,
        prefix: &str,
        tmp: &Path,
    ) -> Result<()> {
        let deflate = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(format!("{prefix}{BUNDLE_MANIFEST}"), deflate)
            .map_err(|e| zip_err(tmp, e))?;
        zip.write_all(&serde_json::to_vec_pretty(&plan.manifest).expect("说明文件可以序列化"))
            .map_err(|e| AppError::io(tmp, e))?;
        // 对象文件已是 zstd 压缩，原样存放。
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        for (kind, ids) in [
            (Kind::Chunk, &plan.chunks),
            (Kind::Blob, &plan.blobs),
            (Kind::Tree, &plan.trees),
            (Kind::Snapshot, &plan.manifest.snapshots),
        ] {
            for id in ids {
                let bytes = self.repo.read_object_file(kind, id)?;
                zip.start_file(
                    format!("{prefix}objects/{}/{}", kind.dir(), id.to_hex()),
                    stored,
                )
                .map_err(|e| zip_err(tmp, e))?;
                zip.write_all(&bytes).map_err(|e| AppError::io(tmp, e))?;
            }
        }
        for sh in &plan.shots {
            let data = self.shot_bytes(sh)?;
            zip.start_file(format!("{prefix}{}", shot_entry(sh)), stored)
                .map_err(|e| zip_err(tmp, e))?;
            zip.write_all(&data).map_err(|e| AppError::io(tmp, e))?;
        }
        Ok(())
    }

    /// 读取历史包并与本地历史比较，不写入任何东西。
    /// `target` 为导入到的本地游戏；省略时按包内的游戏 ID 查找（自定义游戏在不同电脑上的 ID 不同，需要指定）。
    /// 选中的是合集包时，取其中与 `target` 同一 ID 的游戏（只有一个游戏时取它）。
    pub fn inspect_bundle(&self, path: &Path, target: Option<&str>) -> Result<BundlePreview> {
        let mut zip = open_zip(path)?;
        let prefix = self.prefix_for(&mut zip, path, target)?;
        let manifest = read_bundle_manifest(&mut zip, path, &prefix)?;
        let snaps = read_bundle_snapshots(&mut zip, path, &manifest, &prefix)?;
        self.preview_bundle(&manifest, &snaps, target)
    }

    /// 合集包中对应 `target` 游戏的目录；单个历史包为空。
    fn prefix_for(
        &self,
        zip: &mut zip::ZipArchive<std::fs::File>,
        path: &Path,
        target: Option<&str>,
    ) -> Result<String> {
        let Some(c) = read_collection_manifest(zip, path)? else {
            return Ok(String::new());
        };
        if c.games.len() == 1 {
            return Ok(c.games[0].dir.clone());
        }
        let id = match target {
            Some(key) => self.find_game(key)?.id,
            None => String::new(),
        };
        c.games
            .iter()
            .find(|e| e.game_id == id)
            .map(|e| e.dir.clone())
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "这是包含 {} 个游戏的合集包，其中没有这个游戏；请在游戏库中用“导入”选择要导入的游戏",
                    c.games.len()
                ))
            })
    }

    /// 读取历史包或合集包中的全部游戏并与本地比较，不写入任何东西。
    pub fn inspect_archive(&self, path: &Path) -> Result<ArchivePreview> {
        let mut zip = open_zip(path)?;
        let collection = read_collection_manifest(&mut zip, path)?;
        let dirs: Vec<String> = match &collection {
            Some(c) => c.games.iter().map(|e| e.dir.clone()).collect(),
            None => vec![String::new()],
        };
        let mut games = Vec::new();
        for dir in dirs {
            let m = read_bundle_manifest(&mut zip, path, &dir)?;
            let snaps = read_bundle_snapshots(&mut zip, path, &m, &dir)?;
            games.push(ArchiveGame {
                key: dir,
                preview: self.preview_bundle(&m, &snaps, None)?,
            });
        }
        let (created_at, host) = match &collection {
            Some(c) => (c.created_at.clone(), c.host.clone()),
            None => (
                games[0].preview.created_at.clone(),
                games[0].preview.host.clone(),
            ),
        };
        Ok(ArchivePreview {
            collection: collection.is_some(),
            created_at,
            host,
            games,
        })
    }

    /// 导入历史包或合集包中选中的游戏（`picks` 为空表示全部）。每个游戏单独导入，一个失败不影响其他。
    pub fn import_archive(
        &mut self,
        lock: &RepoLock,
        path: &Path,
        picks: &[ArchivePick],
        dry_run: bool,
    ) -> Result<Vec<ArchiveImportItem>> {
        let preview = self.inspect_archive(path)?;
        let picks: Vec<ArchivePick> = if picks.is_empty() {
            preview
                .games
                .iter()
                .map(|g| ArchivePick::all(g.key.clone()))
                .collect()
        } else {
            picks.to_vec()
        };
        let mut out = Vec::new();
        for pick in &picks {
            let game = preview
                .games
                .iter()
                .find(|g| g.key == pick.key)
                .ok_or_else(|| AppError::Invalid(format!("包内没有 {}", pick.key)))?;
            let result = self.import_bundle_at(
                lock,
                path,
                &pick.key,
                pick.target.as_deref(),
                dry_run,
                (pick.saves, pick.shots),
            );
            // 这是我旧电脑的存档：以导入的 main 为本机主存档并整理名称。
            let mut adopted_main = None;
            let mut error = result.as_ref().err().map(|e| e.to_string());
            if !dry_run
                && result.is_ok()
                && pick.adopt_main
                && pick.saves
                && let Some(name) = result
                    .as_ref()
                    .ok()
                    .and_then(|r| r.preview.main_conflict.clone())
            {
                let id = pick
                    .target
                    .clone()
                    .unwrap_or_else(|| game.preview.game_id.clone());
                match self
                    .find_game(&id)
                    .and_then(|g| self.set_primary(lock, &g, &name, true))
                {
                    Ok(r) => adopted_main = Some(r.branch),
                    Err(e) => error = Some(e.to_string()),
                }
            }
            out.push(ArchiveImportItem {
                key: pick.key.clone(),
                game_id: game.preview.game_id.clone(),
                game_name: game.preview.game_name.clone(),
                error,
                report: result.ok(),
                adopted_main,
            });
        }
        Ok(out)
    }

    fn preview_bundle(
        &self,
        m: &BundleManifest,
        snaps: &HashMap<ObjectId, Snapshot>,
        target: Option<&str>,
    ) -> Result<BundlePreview> {
        let local_game = match target {
            Some(key) => Some(self.find_game(key)?),
            None => self
                .find_game(&m.game_id)
                .ok()
                .filter(|g| g.id == m.game_id),
        };
        let has = |id: &ObjectId| self.repo.has(Kind::Snapshot, id);
        let missing: Vec<ObjectId> = m
            .prerequisites
            .iter()
            .filter(|p| !has(p))
            .copied()
            .collect();
        let extra: HashMap<ObjectId, Vec<ObjectId>> = snaps
            .iter()
            .map(|(id, s)| (*id, s.parents.clone()))
            .collect();

        let mut refs = Vec::new();
        let mut taken: HashSet<String> = match &local_game {
            Some(g) => self
                .repo
                .list_refs(&g.id)?
                .into_iter()
                .map(|(r, _)| r.name.to_lowercase())
                .collect(),
            None => HashSet::new(),
        };
        for r in &m.refs {
            let local = match &local_game {
                Some(g) => self.repo.read_ref(
                    &g.id,
                    &RefName {
                        kind: r.kind,
                        name: r.name.clone(),
                    },
                )?,
                None => None,
            };
            let (action, fork_point) = match (r.kind, local) {
                (_, None) => (RefAction::Create, None),
                (_, Some(l)) if l == r.target => (RefAction::Same, None),
                (RefKind::Tag, Some(_)) => (RefAction::TagConflict, None),
                (RefKind::Branch, Some(l)) => {
                    let theirs = self.ancestors(&[r.target], &extra);
                    if theirs.contains(&l) {
                        (RefAction::FastForward, None)
                    } else if self.ancestors(&[l], &HashMap::new()).contains(&r.target) {
                        (RefAction::Contained, None)
                    } else {
                        (RefAction::Diverged, self.fork_point(l, &theirs))
                    }
                }
            };
            let new_name = (action == RefAction::Diverged).then(|| {
                let base = format!("{}-{}", r.name, sanitize_ref(&m.host));
                let mut name = base.clone();
                let mut i = 2;
                while taken.contains(&name.to_lowercase()) {
                    name = format!("{base}-{i}");
                    i += 1;
                }
                taken.insert(name.to_lowercase());
                name
            });
            refs.push(RefPlan {
                kind: r.kind,
                name: r.name.clone(),
                target: r.target,
                action,
                local_target: local,
                new_name,
                fork_point,
            });
        }

        let mut nodes: Vec<BundleNode> = m
            .snapshots
            .iter()
            .filter_map(|id| snaps.get(id).map(|s| (id, s)))
            .map(|(id, s)| BundleNode {
                id: *id,
                parents: s.parents.clone(),
                created_at: s.created_at.clone(),
                trigger: s.trigger,
                comment: m
                    .notes
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| s.comment.clone()),
                files: s.stats.files,
                bytes: s.stats.bytes,
                host: s.host.clone(),
                known: has(id),
            })
            .collect();
        nodes.reverse();
        let create_game = local_game.is_none();
        let local_shots: HashSet<ObjectId> = match &local_game {
            Some(g) => self
                .index
                .shots_of(&g.id)?
                .into_iter()
                .map(|s| s.blob)
                .collect(),
            None => HashSet::new(),
        };
        let location_mismatch = match &local_game {
            Some(g) => {
                let local: Vec<String> = self
                    .locations(&g.id)?
                    .into_iter()
                    .map(|l| l.path_template)
                    .collect();
                let theirs: Vec<String> = m
                    .locations
                    .iter()
                    .map(|l| l.path_template.clone())
                    .collect();
                local != theirs
            }
            None => false,
        };
        let main_conflict = refs
            .iter()
            .find(|r| r.kind == RefKind::Branch && r.name == bonfire_core::refs::DEFAULT_BRANCH)
            .and_then(|r| r.new_name.clone());
        Ok(BundlePreview {
            game_id: local_game
                .as_ref()
                .map(|g| g.id.clone())
                .unwrap_or_else(|| m.game_id.clone()),
            game_name: m.game_name.clone(),
            local_game: local_game.is_some(),
            create_game,
            created_at: m.created_at.clone(),
            host: m.host.clone(),
            new_snapshots: nodes.iter().filter(|n| !n.known).count(),
            nodes,
            missing,
            refs,
            location_mismatch,
            shots: m.shots.len(),
            new_shots: m
                .shots
                .iter()
                .filter(|s| !local_shots.contains(&s.blob))
                .count(),
            main_conflict,
        })
    }

    /// 导入历史包：写入对象并校验，再按预览中的关系更新分支与标签。不移动当前所在的快照，不写入存档目录。
    /// 游戏库中没有这个游戏时加入游戏库（未安装）。选中的是合集包时导入其中与 `target` 对应的游戏。
    pub fn import_bundle(
        &mut self,
        lock: &RepoLock,
        path: &Path,
        target: Option<&str>,
        dry_run: bool,
    ) -> Result<BundleImportReport> {
        let prefix = {
            let mut zip = open_zip(path)?;
            self.prefix_for(&mut zip, path, target)?
        };
        self.import_bundle_at(lock, path, &prefix, target, dry_run, (true, true))
    }

    /// `parts` 为（导入存档历史，导入截图）。
    fn import_bundle_at(
        &mut self,
        lock: &RepoLock,
        path: &Path,
        prefix: &str,
        target: Option<&str>,
        dry_run: bool,
        parts: (bool, bool),
    ) -> Result<BundleImportReport> {
        let (with_saves, with_shots) = parts;
        if !with_saves && !with_shots {
            return Err(AppError::Invalid("请至少选择导入存档或截图".into()));
        }
        let mut zip = open_zip(path)?;
        let mut manifest = read_bundle_manifest(&mut zip, path, prefix)?;
        if !with_saves {
            manifest.refs.clear();
            manifest.snapshots.clear();
            manifest.prerequisites.clear();
        }
        if !with_shots {
            manifest.shots.clear();
        }
        let snaps = read_bundle_snapshots(&mut zip, path, &manifest, prefix)?;
        let mut preview = self.preview_bundle(&manifest, &snaps, target)?;
        if !preview.missing.is_empty() {
            return Err(AppError::Invalid(format!(
                "缺少 {} 个更早的快照（如 {}），这个历史包只包含一段历史，请先导入更早的部分",
                preview.missing.len(),
                &preview.missing[0].to_hex()[..12]
            )));
        }
        if dry_run {
            return Ok(BundleImportReport {
                imported_snapshots: preview.new_snapshots,
                objects: 0,
                created_game: preview.create_game,
                imported_shots: preview.new_shots,
                new_shot_ids: Vec::new(),
                preview,
            });
        }
        let created_game = !preview.local_game;
        if created_game {
            let locations: Vec<(String, LocationKind, FileTag)> = manifest
                .locations
                .iter()
                .map(|l| {
                    (
                        l.path_template.clone(),
                        l.kind,
                        l.tag.unwrap_or(FileTag::Save),
                    )
                })
                .collect();
            let g = self.adopt_game(&manifest.game_id, &manifest.game_name, &locations)?;
            if manifest.shots_only {
                self.set_manage_mode(&g, crate::shots::ManageMode::Shots)?;
            }
            preview = self.preview_bundle(&manifest, &snaps, Some(&g.id))?;
        } else if let Ok(mut g) = self.find_game(&preview.game_id)
            && !g.enabled
        {
            // 导入到发现了但未加入（或已忽略）的游戏：加入游戏库。
            g.enabled = true;
            self.index.upsert_game(&g)?;
            self.save_games_file()?;
        }

        // 1. 对象：分块、文件、清单、快照依次写入（最后写快照，中途失败只留下无引用的对象）。
        let names: Vec<String> = zip
            .file_names()
            .filter_map(|n| n.ok().map(|c| c.into_owned()))
            .collect();
        let mut written = 0;
        let mut new_blobs = Vec::new();
        // 不导入存档历史时跳过全部对象。
        let kinds: &[Kind] = if with_saves {
            &[Kind::Chunk, Kind::Blob, Kind::Tree, Kind::Snapshot]
        } else {
            &[]
        };
        for &kind in kinds {
            let dir = format!("{prefix}objects/{}/", kind.dir());
            for name in names.iter().filter(|n| n.starts_with(&dir)) {
                let id: ObjectId = name[dir.len()..]
                    .parse()
                    .map_err(|_| AppError::Invalid(format!("历史包中的对象名无效：{name}")))?;
                let bytes = read_entry(&mut zip, name, path)?;
                if self.repo.import_object_file(kind, &id, &bytes)? {
                    written += 1;
                    if kind == Kind::Blob {
                        new_blobs.push(id);
                    }
                }
            }
        }
        // 2. 校验：包内每个快照的清单与文件都能完整读出（文件哈希逐个核对）。
        for id in &new_blobs {
            self.repo.read_blob_to(id, std::io::sink(), path)?;
        }
        for id in &manifest.snapshots {
            let s = self.repo.get_snapshot(id)?;
            for e in self.repo.get_tree(&s.tree)?.entries {
                if !self.repo.has(Kind::Blob, &e.blob) {
                    return Err(AppError::Invalid(format!(
                        "历史包不完整：缺少文件 {}",
                        e.path
                    )));
                }
            }
        }
        // 3. 索引、备注、分支与标签。
        let game = preview.game_id.clone();
        // 包内快照记录的游戏 ID 与本地不同时登记为别名，重建索引时仍归到本地游戏。
        let foreign: HashSet<&str> = snaps
            .values()
            .map(|s| s.game.as_str())
            .filter(|g| *g != game)
            .collect();
        for g in foreign {
            self.repo.add_alias(lock, &game, g)?;
        }
        let mut imported = 0;
        for id in &manifest.snapshots {
            let s = &snaps[id];
            if !self.index.snapshots_of(&game)?.iter().any(|r| r.id == *id) {
                imported += 1;
            }
            let comment = manifest
                .notes
                .get(id)
                .cloned()
                .unwrap_or_else(|| s.comment.clone());
            if let Some(note) = manifest.notes.get(id)
                && self.repo.note(id)?.is_none()
            {
                self.repo.set_note(lock, id, note)?;
            }
            self.index.add_snapshot_as(id, s, &comment, &game)?;
        }
        let message = format!(
            "导入 {}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        for r in &preview.refs {
            match r.action {
                RefAction::Create => match r.kind {
                    RefKind::Branch => self.repo.create_branch(lock, &game, &r.name, r.target)?,
                    RefKind::Tag => self.repo.create_tag(lock, &game, &r.name, r.target)?,
                },
                RefAction::FastForward => self.repo.update_ref(
                    lock,
                    &game,
                    &RefName::branch(&r.name),
                    r.local_target,
                    r.target,
                    "import",
                    &message,
                )?,
                RefAction::Diverged => self.repo.create_branch(
                    lock,
                    &game,
                    r.new_name.as_deref().expect("分叉时有新分支名"),
                    r.target,
                )?,
                RefAction::Same | RefAction::Contained | RefAction::TagConflict => {}
            }
        }
        // 新加入的游戏：当前分支取 main（包内没有 main 时取第一个分支）。
        if created_game {
            let refs = self.repo.list_refs(&game)?;
            let head = self.repo.head(&game)?;
            let has = |n: &str| {
                refs.iter()
                    .any(|(r, _)| r.kind == RefKind::Branch && r.name == n)
            };
            if !has(&head.branch)
                && let Some((r, _)) = refs.iter().find(|(r, _)| r.kind == RefKind::Branch)
            {
                self.repo.set_head(
                    lock,
                    &game,
                    &bonfire_core::refs::Head {
                        branch: r.name.clone(),
                        detached: None,
                    },
                )?;
            }
        }
        self.index.set_refs(&game, &self.repo.list_refs(&game)?)?;
        // 新加入的游戏：状态与导入的当前快照比较（磁盘上已有相同存档时显示“已是最新”）。
        if created_game {
            self.cache_head_tree(&game)?;
        }
        // 截图相册：原图在包中的 `shots/` 下（早期的包作为文件对象导入），按本地游戏登记；
        // 本地已有（含回收站与彻底删除的）不再导入。
        let mut imported_shots = 0;
        let mut new_shot_ids = Vec::new();
        let local: HashSet<ObjectId> = self
            .index
            .shots_of(&game)?
            .into_iter()
            .map(|s| s.blob)
            .collect();
        for sh in &manifest.shots {
            if local.contains(&sh.blob) {
                continue;
            }
            let entry = format!("{prefix}{}", shot_entry(sh));
            let data = if names.contains(&entry) {
                read_entry(&mut zip, &entry, path)?
            } else if self.repo.has(Kind::Blob, &sh.blob) {
                let mut buf = Vec::new();
                self.repo
                    .read_blob_to(&sh.blob, &mut buf, self.repo.root())?;
                buf
            } else {
                continue;
            };
            if ObjectId::of(&data) != sh.blob {
                return Err(AppError::Invalid(format!(
                    "历史包中的截图 {} 内容与记录不符",
                    sh.blob.short(12)
                )));
            }
            let file = self.store_album_file(&game, sh, &data)?;
            if self.index.add_shot(&ShotRecord {
                game_id: game.clone(),
                file,
                deleted_at: None,
                purged: false,
                ..sh.clone()
            })? {
                imported_shots += 1;
                new_shot_ids.push(sh.blob);
            }
        }
        if imported_shots > 0 {
            self.save_album(&game)?;
        }
        Ok(BundleImportReport {
            preview,
            imported_snapshots: imported,
            objects: written,
            created_game,
            imported_shots,
            new_shot_ids,
        })
    }

    /// 把别处导出的游戏加入游戏库（本机没有安装或没有发现时），标为未安装：没有安装目录。
    /// 存档位置按模板在本机展开；需要安装目录（`<base>` 等）或本机不存在的绝对路径暂不确定（路径留空），
    /// 装好游戏、扫描平台时按模板对上原来的位置序号，历史照常可用。
    pub fn adopt_game(
        &mut self,
        id: &str,
        name: &str,
        locations: &[(String, LocationKind, FileTag)],
    ) -> Result<GameRecord> {
        bonfire_core::refs::validate_game_id(id)?;
        if let Some(g) = self.index.game(id)? {
            return Ok(g);
        }
        let (platform, app_id) = match id.split_once('-') {
            Some(("custom", _)) | None => ("custom".to_string(), None),
            Some((p, rest)) => (p.to_string(), Some(rest.to_string())),
        };
        let ctx = bonfire_discovery::paths::PathContext {
            dirs: self.known_dirs.clone(),
            ..Default::default()
        };
        let records: Vec<LocationRecord> = locations
            .iter()
            .map(|(template, kind, tag)| {
                let resolved = if *kind == LocationKind::Registry {
                    bonfire_discovery::registry::to_reg_path(template)
                        .or_else(|| Some(template.clone()))
                } else if template.contains('<') {
                    bonfire_discovery::paths::expand(template, &ctx)
                } else {
                    Path::new(template).exists().then(|| template.clone())
                };
                LocationRecord {
                    path_template: template.clone(),
                    confirmed: resolved.is_some(),
                    resolved_path: resolved.unwrap_or_default(),
                    kind: *kind,
                    tag: *tag,
                    source: "import".into(),
                }
            })
            .collect();
        let record = GameRecord {
            id: id.to_string(),
            name: name.to_string(),
            platform,
            app_id,
            install_dir: None,
            enabled: true,
            include_config: None,
            custom_rule_json: None,
        };
        self.index.upsert_game(&record)?;
        self.index.set_locations(id, &records)?;
        self.save_games_file()?;
        Ok(record)
    }

    // ---------- 导入存档文件 ----------

    /// 导入存档文件（zip 或文件夹）为一个新快照。不写入存档目录。
    pub fn import_files(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        opts: &ImportFilesOptions,
    ) -> Result<ImportFilesReport> {
        if let Some(c) = &opts.comment {
            crate::backup::check_comment(c)?;
        }
        let staging = self.paths.cache_dir.join("import").join(now_compact());
        let result = self.import_files_inner(lock, game, src, opts, &staging);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// 预览导入存档文件：每个文件夹对应哪个存档位置、是否与已有快照相同。
    pub fn inspect_files(&self, game: &GameRecord, src: &Path) -> Result<FilesPreview> {
        let staging = self
            .paths
            .cache_dir
            .join("import")
            .join(format!("inspect-{}", now_compact()));
        let result = (|| {
            let (files, manifest) = gather_source(src, &staging)?;
            let (preview, _) = self.map_files(game, &files, manifest)?;
            Ok(preview)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// 读取存档文件附带的说明（篝火导出的 zip 或文件夹），找出属于游戏库中的哪个游戏。
    pub fn inspect_files_source(&self, src: &Path) -> Result<FilesSource> {
        let staging = self
            .paths
            .cache_dir
            .join("import")
            .join(format!("source-{}", now_compact()));
        let result = (|| {
            let (files, manifest) = gather_source(src, &staging)?;
            let manifest = manifest.ok_or_else(|| {
                AppError::Invalid(
                    "这些存档文件没有篝火的导出说明，无法判断属于哪个游戏；请在对应游戏的详情页中导入".into(),
                )
            })?;
            let game = self
                .index
                .game(&manifest.game_id)?
                .or_else(|| self.find_game(&manifest.game_name).ok());
            let preview = match &game {
                Some(g) => Some(self.map_files(g, &files, Some(manifest.clone()))?.0),
                None => None,
            };
            Ok(FilesSource {
                game_id: game.map(|g| g.id),
                manifest,
                preview,
            })
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// 在游戏库中导入存档文件：游戏库中没有这个游戏时新建（未安装），放在 main 分支上；
    /// 已有时放在一个新分支上（与详情页导入的默认做法相同）。
    pub fn import_files_auto(
        &mut self,
        lock: &RepoLock,
        src: &Path,
        comment: Option<String>,
        dry_run: bool,
    ) -> Result<FilesAutoReport> {
        let source = self.inspect_files_source(src)?;
        let m = &source.manifest;
        let (game, created_game) = match &source.game_id {
            Some(id) => (self.find_game(id)?, false),
            None if dry_run => {
                return Err(AppError::Invalid(format!(
                    "游戏库中没有「{}」，导入时会新建",
                    m.game_name
                )));
            }
            None => {
                let locations: Vec<(String, LocationKind, FileTag)> = m
                    .locations
                    .iter()
                    .map(|l| (l.path_template.clone(), l.kind, FileTag::Save))
                    .collect();
                (self.adopt_game(&m.game_id, &m.game_name, &locations)?, true)
            }
        };
        let place = if self.repo.list_refs(&game.id)?.is_empty() {
            ImportPlace::NewBranch {
                name: Some(self.repo.head(&game.id)?.branch),
                parent: None,
            }
        } else {
            ImportPlace::default()
        };
        let report = self.import_files(
            lock,
            &game,
            src,
            &ImportFilesOptions {
                place,
                comment,
                dry_run,
            },
        )?;
        if created_game {
            self.cache_head_tree(&game.id)?;
        }
        Ok(FilesAutoReport {
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            created_game,
            report,
        })
    }

    fn import_files_inner(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        opts: &ImportFilesOptions,
        staging: &Path,
    ) -> Result<ImportFilesReport> {
        let (files, manifest) = gather_source(src, staging)?;
        let (mut preview, mapped) = self.map_files(game, &files, manifest)?;
        if !preview.unmapped.is_empty() {
            return Err(AppError::Invalid(format!(
                "有 {} 个文件无法对应到「{}」的存档位置（如 {}）；请按导出时的文件夹结构放置，每个存档位置一个文件夹",
                preview.unmapped.len(),
                game.name,
                preview.unmapped[0]
            )));
        }
        if mapped.is_empty() {
            return Err(AppError::Invalid("没有找到存档文件".into()));
        }
        if preview.identical.is_some() || opts.dry_run {
            return Ok(ImportFilesReport {
                preview,
                snapshot: None,
                branch: None,
                dry_run: opts.dry_run,
            });
        }

        // 存入对象库，生成清单。
        let mut entries = Vec::with_capacity(mapped.len());
        for (tree_path, abs) in &mapped {
            let f = std::fs::File::open(abs).map_err(|e| AppError::io(abs, e))?;
            let meta = f.metadata().map_err(|e| AppError::io(abs, e))?;
            let stats = self.repo.write_file(&f, abs)?;
            entries.push(TreeEntry {
                path: tree_path.clone(),
                blob: stats.blob,
                size: stats.size,
                mtime_ns: scan::mtime_ns(&meta),
                tag: bonfire_core::object::FileTag::Save,
            });
        }
        let tree = Tree::new(entries);
        let tree_id = self.repo.put_tree(&tree)?;
        if let Some(id) = self.snapshot_with_tree(&game.id, &tree_id)? {
            preview.identical = Some(id);
            return Ok(ImportFilesReport {
                preview,
                snapshot: None,
                branch: None,
                dry_run: false,
            });
        }

        let (parent, branch, expected) = match &opts.place {
            ImportPlace::NewBranch { name, parent } => {
                let parent = match parent {
                    Some(p) => Some(self.repo.resolve(&game.id, p)?),
                    None => None,
                };
                let name = match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    Some(n) => n.to_string(),
                    None => format!("imported-{}", now_compact()),
                };
                bonfire_core::refs::validate_ref_name(&name)?;
                (parent, name, None)
            }
            ImportPlace::Continue { branch } => {
                let tip = self
                    .repo
                    .read_ref(&game.id, &RefName::branch(branch))?
                    .ok_or_else(|| AppError::Invalid(format!("找不到分支「{branch}」")))?;
                (Some(tip), branch.clone(), Some(tip))
            }
        };
        let source_name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let comment = match opts
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(c) => c.to_string(),
            None => match preview
                .manifest
                .as_ref()
                .and_then(|m| m.comment.clone())
                .filter(|c| !c.is_empty())
            {
                Some(c) => format!("导入：{source_name}（{c}）"),
                None => format!("导入：{source_name}"),
            },
        };
        let total = tree.total_size();
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
                bytes: total,
                changed: tree.entries.len() as u64,
            },
            playtime_min: None,
        };
        let id = self.repo.put_snapshot(&snap)?;
        match expected {
            Some(tip) => self.repo.update_ref(
                lock,
                &game.id,
                &RefName::branch(&branch),
                Some(tip),
                id,
                "import",
                &comment,
            )?,
            None => self.repo.create_branch(lock, &game.id, &branch, id)?,
        }
        self.index.add_snapshot(&id, &snap, &comment)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        Ok(ImportFilesReport {
            preview,
            snapshot: Some(id),
            branch: Some(branch),
            dry_run: false,
        })
    }

    /// 把来源中的文件对应到本地存档位置，返回预览与（清单路径, 文件）列表。
    fn map_files(
        &self,
        game: &GameRecord,
        files: &[(String, PathBuf)],
        manifest: Option<ExportManifest>,
    ) -> Result<(FilesPreview, Vec<(String, PathBuf)>)> {
        let locations = self.locations(&game.id)?;
        if locations.is_empty() {
            return Err(AppError::NoLocations(game.name.clone()));
        }
        let folders = location_folders(&locations);
        // 来源中的顶层文件夹 → 本地存档位置序号。
        let mut by_folder: HashMap<String, usize> = HashMap::new();
        if let Some(m) = &manifest {
            for l in &m.locations {
                let ord = locations
                    .iter()
                    .position(|x| x.path_template == l.path_template)
                    .or_else(|| {
                        (l.ord < locations.len() && m.locations.len() == locations.len())
                            .then_some(l.ord)
                    });
                if let Some(ord) = ord {
                    by_folder.insert(l.folder.to_lowercase(), ord);
                }
            }
        }
        for (ord, f) in folders.iter().enumerate() {
            by_folder.entry(f.to_lowercase()).or_insert(ord);
        }
        let top = |rel: &str| rel.split_once('/').map(|(a, _)| a.to_string());
        // 没有任何顶层文件夹能对应、且只有一个存档位置时，整个来源就是这个位置的内容。
        let none_match = files
            .iter()
            .all(|(rel, _)| top(rel).is_none_or(|t| !by_folder.contains_key(&t.to_lowercase())));
        let single = none_match && locations.len() == 1;

        let mut mapped = Vec::new();
        let mut unmapped = Vec::new();
        let mut stats: BTreeMap<String, (Option<usize>, u64)> = BTreeMap::new();
        let mut bytes = 0;
        for (rel, abs) in files {
            bytes += std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
            let (folder, ord, inner) = if single {
                (String::new(), Some(0), rel.clone())
            } else {
                match rel.split_once('/') {
                    Some((t, rest)) => (
                        t.to_string(),
                        by_folder.get(&t.to_lowercase()).copied(),
                        rest.to_string(),
                    ),
                    None => (String::new(), None, rel.clone()),
                }
            };
            let entry = stats.entry(folder).or_insert((ord, 0));
            entry.1 += 1;
            match ord {
                Some(o) => mapped.push((format!("{o}/{inner}"), abs.clone())),
                None => unmapped.push(rel.clone()),
            }
        }
        // 内容是否与已有快照完全相同：按文件哈希比较。
        let identical = if unmapped.is_empty() && !mapped.is_empty() {
            self.identical_snapshot(&game.id, &mapped)?
        } else {
            None
        };
        let preview = FilesPreview {
            files: files.len() as u64,
            bytes,
            folders: stats
                .into_iter()
                .map(|(folder, (ord, files))| FolderMap { folder, ord, files })
                .collect(),
            unmapped,
            manifest,
            identical,
        };
        Ok((preview, mapped))
    }

    pub(crate) fn identical_snapshot(
        &self,
        game_id: &str,
        mapped: &[(String, PathBuf)],
    ) -> Result<Option<ObjectId>> {
        let mut want: BTreeMap<&str, (u64, Option<ObjectId>)> = BTreeMap::new();
        for (p, abs) in mapped {
            want.insert(
                p.as_str(),
                (std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0), None),
            );
        }
        let mut hashed: HashMap<&str, ObjectId> = HashMap::new();
        for row in self.index.snapshots_of(game_id)? {
            let tree = self.repo.get_tree(&self.repo.get_snapshot(&row.id)?.tree)?;
            if tree.entries.len() != want.len()
                || !tree
                    .entries
                    .iter()
                    .all(|e| want.get(e.path.as_str()).is_some_and(|(s, _)| *s == e.size))
            {
                continue;
            }
            let mut same = true;
            for e in &tree.entries {
                let h = match hashed.get(e.path.as_str()) {
                    Some(h) => *h,
                    None => {
                        let abs = &mapped
                            .iter()
                            .find(|(p, _)| *p == e.path)
                            .expect("路径已比对")
                            .1;
                        let h = scan::hash_file(abs)?;
                        hashed.insert(
                            mapped
                                .iter()
                                .find(|(p, _)| *p == e.path)
                                .expect("路径已比对")
                                .0
                                .as_str(),
                            h,
                        );
                        h
                    }
                };
                if h != e.blob {
                    same = false;
                    break;
                }
            }
            if same {
                return Ok(Some(row.id));
            }
        }
        Ok(None)
    }

    pub(crate) fn snapshot_with_tree(
        &self,
        game_id: &str,
        tree: &ObjectId,
    ) -> Result<Option<ObjectId>> {
        for row in self.index.snapshots_of(game_id)? {
            if self.repo.get_snapshot(&row.id)?.tree == *tree {
                return Ok(Some(row.id));
            }
        }
        Ok(None)
    }

    /// 从若干快照出发能到达的全部快照（含自身）。`extra` 中的父子关系优先（尚未写入仓库的历史包快照）；
    /// 本地没有的快照到此为止。
    pub(crate) fn ancestors(
        &self,
        starts: &[ObjectId],
        extra: &HashMap<ObjectId, Vec<ObjectId>>,
    ) -> HashSet<ObjectId> {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<ObjectId> = starts.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) {
                continue;
            }
            let parents = match extra.get(&id) {
                Some(p) => p.clone(),
                None => match self.repo.get_snapshot(&id) {
                    Ok(s) => s.parents,
                    Err(_) => {
                        seen.remove(&id);
                        continue;
                    }
                },
            };
            queue.extend(parents);
        }
        seen
    }

    /// 本地分支 `local` 与对方历史 `theirs` 的分叉点：沿本地历史由近及远找到的第一个双方共有的快照。
    fn fork_point(&self, local: ObjectId, theirs: &HashSet<ObjectId>) -> Option<ObjectId> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([local]);
        while let Some(id) = queue.pop_front() {
            if theirs.contains(&id) {
                return Some(id);
            }
            if seen.insert(id)
                && let Ok(s) = self.repo.get_snapshot(&id)
            {
                queue.extend(s.parents);
            }
        }
        None
    }
}

/// 每个存档位置在导出中的文件夹名：取位置路径的最后一段，重名时加序号。
pub fn location_folders(locations: &[LocationRecord]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    locations
        .iter()
        .enumerate()
        .map(|(ord, l)| {
            let base = match l.kind {
                LocationKind::Registry => "registry".to_string(),
                LocationKind::File => Path::new(&l.resolved_path)
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                LocationKind::Glob => glob_base(&l.resolved_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                LocationKind::Dir => Path::new(&l.resolved_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            };
            let mut name = sanitize(&base);
            if name.is_empty() {
                name = format!("location-{ord}");
            }
            if !used.insert(name.to_lowercase()) {
                name = format!("{name}-{ord}");
                used.insert(name.to_lowercase());
            }
            name
        })
        .collect()
}

/// 来源中的全部文件：(相对路径, 磁盘文件)，以及附带的说明。zip 先解压到 `staging`。
fn gather_source(src: &Path, staging: &Path) -> Result<(SourceFiles, Option<ExportManifest>)> {
    let meta = std::fs::metadata(src).map_err(|e| AppError::io(src, e))?;
    let root = if meta.is_dir() {
        src.to_path_buf()
    } else {
        let mut zip = open_zip(src)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| zip_err(src, e))?;
            if entry.is_dir() {
                continue;
            }
            // 拒绝跳出解压目录的路径（zip slip）。
            let Some(rel) = entry.enclosed_name() else {
                return Err(AppError::Invalid(format!(
                    "压缩包中有不安全的路径：{}",
                    entry.name().unwrap_or_default()
                )));
            };
            let out = staging.join(rel);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            let mut f = std::fs::File::create(&out).map_err(|e| AppError::io(&out, e))?;
            std::io::copy(&mut entry, &mut f).map_err(|e| AppError::io(&out, e))?;
        }
        staging.to_path_buf()
    };
    // 来源只有一个顶层文件夹且其中有说明文件时（解压出的导出文件夹），以它为根。
    let root = match single_child_with_manifest(&root) {
        Some(inner) => inner,
        None => root,
    };
    let manifest = std::fs::read(root.join(EXPORT_MANIFEST))
        .ok()
        .and_then(|b| serde_json::from_slice::<ExportManifest>(&b).ok())
        .filter(|m| m.format == FILES_FORMAT);
    let mut files = Vec::new();
    for e in walkdir_files(&root)? {
        let rel = e
            .strip_prefix(&root)
            .expect("在根目录下")
            .to_string_lossy()
            .replace('\\', "/");
        if rel == EXPORT_MANIFEST || rel.eq_ignore_ascii_case(crate::branding::README_NAME) {
            continue;
        }
        files.push((rel, e));
    }
    files.sort();
    Ok((files, manifest))
}

fn single_child_with_manifest(root: &Path) -> Option<PathBuf> {
    if root.join(EXPORT_MANIFEST).is_file() {
        return None;
    }
    let entries: Vec<_> = std::fs::read_dir(root).ok()?.flatten().collect();
    match entries.as_slice() {
        [only] if only.path().is_dir() && only.path().join(EXPORT_MANIFEST).is_file() => {
            Some(only.path())
        }
        _ => None,
    }
}

fn walkdir_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)
            .map_err(|err| AppError::io(&dir, err))?
            .flatten()
        {
            let p = e.path();
            let ft = e.file_type().map_err(|err| AppError::io(&p, err))?;
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                out.push(p);
            }
        }
    }
    Ok(out)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).map_err(|e| AppError::io(path, e))?;
    zip::ZipArchive::new(f)
        .map_err(|e| AppError::Invalid(format!("{} 不是有效的压缩包：{e}", path.display())))
}

fn read_entry(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    path: &Path,
) -> Result<Vec<u8>> {
    let mut e = zip.by_name(name).map_err(|e| zip_err(path, e))?;
    let mut buf = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut buf)
        .map_err(|err| AppError::io(path, err))?;
    Ok(buf)
}

/// 合集包的说明；不是合集包时为 None。
fn read_collection_manifest(
    zip: &mut zip::ZipArchive<std::fs::File>,
    path: &Path,
) -> Result<Option<CollectionManifest>> {
    if zip.index_for_name(COLLECTION_MANIFEST).is_none() {
        return Ok(None);
    }
    let bytes = read_entry(zip, COLLECTION_MANIFEST, path)?;
    let c: CollectionManifest = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Invalid(format!("合集包说明无法读取：{e}")))?;
    if c.format != COLLECTION_FORMAT || c.version != 1 {
        return Err(AppError::Invalid(format!(
            "不支持的合集包格式：{} v{}",
            c.format, c.version
        )));
    }
    if c.games
        .iter()
        .any(|e| !e.dir.starts_with("games/") || e.dir.contains(".."))
    {
        return Err(AppError::Invalid("合集包说明中的目录无效".into()));
    }
    Ok(Some(c))
}

fn read_bundle_manifest(
    zip: &mut zip::ZipArchive<std::fs::File>,
    path: &Path,
    prefix: &str,
) -> Result<BundleManifest> {
    let bytes = read_entry(zip, &format!("{prefix}{BUNDLE_MANIFEST}"), path)
        .map_err(|_| AppError::Invalid(format!("{} 不是篝火历史包", path.display())))?;
    let m: BundleManifest = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Invalid(format!("历史包说明无法读取：{e}")))?;
    if m.format != BUNDLE_FORMAT || m.version != 1 {
        return Err(AppError::Invalid(format!(
            "不支持的历史包格式：{} v{}",
            m.format, m.version
        )));
    }
    Ok(m)
}

/// 读取并校验包内全部快照对象。
fn read_bundle_snapshots(
    zip: &mut zip::ZipArchive<std::fs::File>,
    path: &Path,
    m: &BundleManifest,
    prefix: &str,
) -> Result<HashMap<ObjectId, Snapshot>> {
    let mut out = HashMap::new();
    for id in &m.snapshots {
        let stored = read_entry(
            zip,
            &format!("{prefix}objects/{}/{}", Kind::Snapshot.dir(), id.to_hex()),
            path,
        )?;
        let raw = zstd::stream::decode_all(&stored[..])
            .map_err(|e| AppError::Invalid(format!("历史包中的快照 {id} 已损坏：{e}")))?;
        if ObjectId::of(&raw) != *id {
            return Err(AppError::Invalid(format!(
                "历史包中的快照 {id} 已损坏：内容与 ID 不符"
            )));
        }
        let s: Snapshot = serde_json::from_slice(&raw)
            .map_err(|e| AppError::Invalid(format!("历史包中的快照 {id} 无法读取：{e}")))?;
        out.insert(*id, s);
    }
    Ok(out)
}

/// 父快照在前的顺序。
fn topo_order(snaps: &HashMap<ObjectId, Snapshot>) -> Vec<ObjectId> {
    let mut ids: Vec<&ObjectId> = snaps.keys().collect();
    ids.sort_by(|a, b| snaps[a].created_at.cmp(&snaps[b].created_at).then(a.cmp(b)));
    let mut done: HashSet<ObjectId> = HashSet::new();
    let mut order = Vec::with_capacity(ids.len());
    for &start in &ids {
        let mut stack = vec![*start];
        while let Some(&id) = stack.last() {
            if done.contains(&id) {
                stack.pop();
                continue;
            }
            let pending: Vec<ObjectId> = snaps[&id]
                .parents
                .iter()
                .filter(|p| snaps.contains_key(p) && !done.contains(p))
                .copied()
                .collect();
            if pending.is_empty() {
                done.insert(id);
                order.push(id);
                stack.pop();
            } else {
                stack.extend(pending);
            }
        }
    }
    order
}

/// 文件名中不能出现的字符替换为 `_`。
fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    out.trim().trim_end_matches('.').to_string()
}

/// 用作分支名的一段：只保留字母、数字、`-`、`_`。
fn sanitize_ref(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "imported".into()
    } else {
        out
    }
}

pub(crate) fn path_key(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    abs.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

pub(crate) fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut name = p.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    p.with_file_name(name)
}

/// `dir` 下一个尚不存在的子路径：`name`、`name (2)`……
pub(crate) fn unique_child(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.join(name);
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{name} ({i})"));
        i += 1;
    }
    p
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(AppError::Invalid(format!("无效的路径 {rel}")));
        }
        p.push(part);
    }
    Ok(p)
}

pub(crate) fn replace_file(tmp: &Path, dest: &Path) -> Result<()> {
    if let Some(dir) = dest.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
    }
    std::fs::rename(tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        AppError::io(dest, e)
    })
}

/// 历史包中截图原图的位置：`shots/<内容哈希>.<扩展名>`。
fn shot_entry(s: &ShotRecord) -> String {
    let ext = Path::new(&s.original_path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "img".into());
    format!("shots/{}.{ext}", s.blob.to_hex())
}

pub(crate) fn zip_err(path: &Path, e: zip::result::ZipError) -> AppError {
    AppError::io(path, std::io::Error::other(e))
}

/// zip 中的文件时间：快照时间（导出快照时）或当前时间。
pub(crate) fn zip_time(rfc3339: Option<&str>) -> zip::DateTime {
    let t = rfc3339
        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()))
        .unwrap_or_else(jiff::Zoned::now);
    zip::DateTime::from_date_and_time(
        t.year() as u16,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackupOptions;
    use crate::testutil::Env;

    fn backup(env: &mut Env, g: &GameRecord, comment: &str) -> ObjectId {
        let lock = env.app.repo.lock().unwrap();
        match env
            .app
            .backup(
                &lock,
                g,
                &BackupOptions {
                    comment: Some(comment.into()),
                    ..Default::default()
                },
            )
            .unwrap()
        {
            crate::BackupResult::Created { snapshot, .. } => snapshot,
            other => panic!("{other:?}"),
        }
    }

    fn zip_names(p: &Path) -> Vec<String> {
        let z = open_zip(p).unwrap();
        let mut v: Vec<String> = z
            .file_names()
            .filter_map(|n| n.ok().map(|c| c.into_owned()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn export_snapshot_and_current_as_zip_and_folder() {
        let mut env = Env::new();
        env.write("slot1/a.sav", b"one");
        let g = env.add_game("Hollow Knight");
        let first = backup(&mut env, &g, "第一次");
        env.write("slot1/a.sav", b"two!");
        let out = env.dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        // 导出旧快照：内容是快照时的，不是磁盘上的。
        let zip_path = out.join("old.zip");
        let r = env
            .app
            .export_files(&g, Some(&first.to_string()), &zip_path, ExportFormat::Zip)
            .unwrap();
        assert_eq!((r.files, r.snapshot), (2, Some(first)));
        let names = zip_names(&zip_path);
        // 存档 2 个 + 说明文件 + readme.txt（U-47）。
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(names.contains(&"readme.txt".to_string()), "{names:?}");
        assert!(
            names.contains(&"bonfire-export.json".to_string())
                && names.contains(&"saves/slot1/a.sav".to_string()),
            "{names:?}"
        );
        let mut z = open_zip(&zip_path).unwrap();
        assert_eq!(
            read_entry(&mut z, "saves/slot1/a.sav", &zip_path).unwrap(),
            b"one"
        );

        // 导出当前存档到文件夹：在上级目录中新建文件夹。
        let r = env
            .app
            .export_files(&g, None, &out, ExportFormat::Folder)
            .unwrap();
        assert!(
            r.dest.starts_with(&out) && r.dest.is_dir(),
            "{}",
            r.dest.display()
        );
        assert_eq!(
            std::fs::read(r.dest.join("saves/slot1/a.sav")).unwrap(),
            b"two!"
        );
        let m: ExportManifest =
            serde_json::from_slice(&std::fs::read(r.dest.join(EXPORT_MANIFEST)).unwrap()).unwrap();
        assert_eq!((m.game_id.as_str(), m.snapshot), (g.id.as_str(), None));
        // 篝火标识：文件夹名、generator、readme.txt。
        let dir_name = r.dest.file_name().unwrap().to_string_lossy().into_owned();
        assert!(dir_name.starts_with("Bonfire-"), "{dir_name}");
        assert!(crate::branding::is_bonfire_generator(
            m.generator.as_deref()
        ));
        let readme = std::fs::read_to_string(r.dest.join("readme.txt")).unwrap();
        assert!(crate::branding::is_bonfire_readme(&readme) && readme.contains("gitee.com"));
        // 再导入这个文件夹：readme.txt 不影响对应存档位置。
        let p = env.app.inspect_files(&g, &r.dest).unwrap();
        assert!(p.unmapped.is_empty(), "{:?}", p.unmapped);
        // 拖进“导入下载的存档”：识别为篝火导出的。
        assert_eq!(
            env.app.inspect_download(&g, &r.dest, None).unwrap().kind,
            crate::downloads::DownloadKind::Bonfire
        );
    }

    #[test]
    fn export_refuses_save_dirs_and_repo() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        backup(&mut env, &g, "x");
        let err = env
            .app
            .export_files(&g, None, &env.saves().join("x.zip"), ExportFormat::Zip)
            .unwrap_err();
        assert!(err.to_string().contains("存档位置"), "{err}");
        let err = env
            .app
            .export_files(
                &g,
                None,
                &env.app.repo.root().join("x.zip"),
                ExportFormat::Zip,
            )
            .unwrap_err();
        assert!(err.to_string().contains("仓库"), "{err}");
        assert!(!env.saves().join("x.zip").exists());
    }

    #[test]
    fn import_exported_files_as_new_branch_and_detect_identical() {
        let mut env = Env::new();
        env.write("a.sav", b"mine");
        let g = env.add_game("Game");
        let first = backup(&mut env, &g, "mine");
        let out = env.dir.path().join("share.zip");
        env.app
            .export_files(&g, Some("main"), &out, ExportFormat::Zip)
            .unwrap();

        // 同样的内容：不重复生成快照。
        let lock = env.app.repo.lock().unwrap();
        let r = env
            .app
            .import_files(&lock, &g, &out, &ImportFilesOptions::default())
            .unwrap();
        assert_eq!((r.snapshot, r.preview.identical), (None, Some(first)));

        // 别人的存档（改过的内容）：新分支、独立起点，不移动 HEAD，不写存档目录。
        let friend = env.dir.path().join("friend");
        std::fs::create_dir_all(friend.join("saves")).unwrap();
        std::fs::write(friend.join("saves/a.sav"), b"friend's").unwrap();
        let r = env
            .app
            .import_files(&lock, &g, &friend, &ImportFilesOptions::default())
            .unwrap();
        let id = r.snapshot.unwrap();
        assert!(r.branch.as_deref().unwrap().starts_with("imported-"));
        let s = env.app.repo.get_snapshot(&id).unwrap();
        assert!(s.parents.is_empty());
        assert_eq!(s.trigger, Trigger::Import);
        assert_eq!(env.app.repo.head_snapshot(&g.id).unwrap(), Some(first));
        assert_eq!(env.read("a.sav").unwrap(), b"mine");

        // 接在 main 之后：main 前移。
        std::fs::write(friend.join("saves/a.sav"), b"friend v2").unwrap();
        let opts = ImportFilesOptions {
            place: ImportPlace::Continue {
                branch: "main".into(),
            },
            ..Default::default()
        };
        let r = env.app.import_files(&lock, &g, &friend, &opts).unwrap();
        let tip = env
            .app
            .repo
            .read_ref(&g.id, &RefName::branch("main"))
            .unwrap();
        assert_eq!(tip, r.snapshot);
        assert_eq!(
            env.app.repo.get_snapshot(&tip.unwrap()).unwrap().parents,
            vec![first]
        );
    }

    #[test]
    fn import_files_reports_unmapped_folders() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let src = env.dir.path().join("odd");
        std::fs::create_dir_all(src.join("whatever")).unwrap();
        std::fs::write(src.join("whatever/a.sav"), b"x").unwrap();
        let p = env.app.inspect_files(&g, &src).unwrap();
        // 测试游戏有两个位置（saves 与 cfg.ini），对应不上的文件夹要报告。
        assert_eq!(p.unmapped, ["whatever/a.sav"]);
        let lock = env.app.repo.lock().unwrap();
        assert!(
            env.app
                .import_files(&lock, &g, &src, &ImportFilesOptions::default())
                .is_err()
        );
    }

    /// 两个仓库模拟两台电脑：A 导出，B 导入。
    #[test]
    fn bundle_roundtrip_fast_forward_contained_and_diverged() {
        let mut a = Env::new();
        a.write("a.sav", b"v1");
        let g = a.add_game("Game");
        let s1 = backup(&mut a, &g, "v1");
        let out = a.dir.path().join("all.bonfire");
        let r = a
            .app
            .export_bundle(&g, &BundleScope::All, &out, false)
            .unwrap();
        assert_eq!((r.snapshots, r.prerequisites), (1, 0));

        let mut b = Env::new();
        b.write("a.sav", b"other");
        // 自定义游戏在另一台电脑上的 ID 不同：导入时指定本地游戏。
        let gb = b.add_game("Game");
        assert_ne!(gb.id, g.id);
        // B 本地没有 main：导入后新建。
        let lock = b.app.repo.lock().unwrap();
        let p = b.app.inspect_bundle(&out, Some(&gb.id)).unwrap();
        assert_eq!(p.refs[0].action, RefAction::Create);
        b.app
            .import_bundle(&lock, &out, Some(&gb.id), false)
            .unwrap();
        assert_eq!(
            b.app
                .repo
                .read_ref(&gb.id, &RefName::branch("main"))
                .unwrap(),
            Some(s1)
        );
        // 再导入一次：相同。
        assert_eq!(
            b.app.inspect_bundle(&out, Some(&gb.id)).unwrap().refs[0].action,
            RefAction::Same
        );
        drop(lock);

        // A 继续玩：B 导入后是后续更新（快进）。
        a.write("a.sav", b"v2");
        let s2 = backup(&mut a, &g, "v2");
        let seg = a.dir.path().join("seg.bonfire");
        let r = a
            .app
            .export_bundle(
                &g,
                &BundleScope::Range {
                    from: Some(s2.to_string()),
                    to: "main".into(),
                },
                &seg,
                false,
            )
            .unwrap();
        assert_eq!((r.snapshots, r.prerequisites), (1, 1));
        let lock = b.app.repo.lock().unwrap();
        let p = b.app.inspect_bundle(&seg, Some(&gb.id)).unwrap();
        assert_eq!(p.refs[0].action, RefAction::FastForward);
        b.app
            .import_bundle(&lock, &seg, Some(&gb.id), false)
            .unwrap();
        assert_eq!(
            b.app
                .repo
                .read_ref(&gb.id, &RefName::branch("main"))
                .unwrap(),
            Some(s2)
        );
        // 旧的包对 B 来说已包含。
        assert_eq!(
            b.app.inspect_bundle(&out, Some(&gb.id)).unwrap().refs[0].action,
            RefAction::Contained
        );
        drop(lock);

        // 双方各自继续：分叉，另建分支，不覆盖 B 的 main。
        a.write("a.sav", b"a3");
        backup(&mut a, &g, "a3");
        // B 的 main 已快进到 s2，B 在其上继续备份。
        b.write("a.sav", b"b3");
        let b3 = backup(&mut b, &gb, "b3");
        let all = a.dir.path().join("all2.bonfire");
        a.app
            .export_bundle(&g, &BundleScope::All, &all, false)
            .unwrap();
        let lock = b.app.repo.lock().unwrap();
        let p = b.app.inspect_bundle(&all, Some(&gb.id)).unwrap();
        let main = p.refs.iter().find(|r| r.name == "main").unwrap();
        assert_eq!(
            (main.action, main.fork_point),
            (RefAction::Diverged, Some(s2))
        );
        let r = b
            .app
            .import_bundle(&lock, &all, Some(&gb.id), false)
            .unwrap();
        assert_eq!(r.imported_snapshots, 1);
        assert_eq!(
            b.app
                .repo
                .read_ref(&gb.id, &RefName::branch("main"))
                .unwrap(),
            Some(b3)
        );
        let new_name = main.new_name.clone().unwrap();
        assert!(
            b.app
                .repo
                .read_ref(&gb.id, &RefName::branch(&new_name))
                .unwrap()
                .is_some()
        );
        // 导入的快照能完整读出。
        assert_eq!(b.app.history_view(&gb).unwrap().nodes.len(), 4);
        // 重建索引后导入的快照仍归在本地游戏下。
        b.app.reindex(&lock).unwrap();
        assert_eq!(b.app.history_view(&gb).unwrap().nodes.len(), 4);
    }

    #[test]
    fn segment_needs_prerequisites() {
        let mut a = Env::new();
        a.write("a.sav", b"v1");
        let g = a.add_game("Game");
        backup(&mut a, &g, "v1");
        a.write("a.sav", b"v2");
        let s2 = backup(&mut a, &g, "v2");
        let seg = a.dir.path().join("seg.bonfire");
        a.app
            .export_bundle(
                &g,
                &BundleScope::Range {
                    from: Some(s2.to_string()),
                    to: s2.to_string(),
                },
                &seg,
                false,
            )
            .unwrap();

        let mut b = Env::new();
        b.write("a.sav", b"x");
        let gb = b.add_game("Game");
        let p = b.app.inspect_bundle(&seg, Some(&gb.id)).unwrap();
        assert_eq!(p.missing.len(), 1);
        let lock = b.app.repo.lock().unwrap();
        let err = b
            .app
            .import_bundle(&lock, &seg, Some(&gb.id), false)
            .unwrap_err();
        assert!(err.to_string().contains("更早"), "{err}");
    }

    /// 合集包：一次导出多个游戏；导入选中的游戏，本地没有的游戏加入游戏库（未安装）。
    #[test]
    fn collection_export_and_import_creates_missing_games() {
        let mut a = Env::new();
        a.write("a.sav", b"v1");
        let g1 = a.add_game("One");
        backup(&mut a, &g1, "one");
        let g2 = a.add_game("Two");
        backup(&mut a, &g2, "two");
        let g3 = a.add_game("Empty");
        let out = a.dir.path().join("all.bonfire");
        let r = a
            .app
            .export_collection(&[g1.clone(), g2.clone(), g3], &out, false)
            .unwrap();
        assert_eq!(
            (r.games.len(), r.skipped.clone()),
            (2, vec!["Empty".to_string()])
        );
        // A 自己再看：两个游戏都已有，内容相同。
        let p = a.app.inspect_archive(&out).unwrap();
        assert!(p.collection);
        assert!(
            p.games
                .iter()
                .all(|g| g.preview.local_game && g.preview.new_snapshots == 0)
        );
        // 在游戏详情中选中合集包：取对应的游戏。
        assert_eq!(
            a.app.inspect_bundle(&out, Some(&g2.id)).unwrap().game_name,
            "Two"
        );

        let mut b = Env::new();
        let p = b.app.inspect_archive(&out).unwrap();
        assert!(p.games.iter().all(|g| g.preview.create_game));
        let key = p
            .games
            .iter()
            .find(|g| g.preview.game_name == "One")
            .unwrap()
            .key
            .clone();
        let lock = b.app.repo.lock().unwrap();
        let items = b
            .app
            .import_archive(&lock, &out, &[ArchivePick::all(key)], false)
            .unwrap();
        assert_eq!(items.len(), 1);
        let rep = items[0].report.as_ref().unwrap();
        assert!(rep.created_game && rep.imported_snapshots == 1, "{items:?}");
        let g = b.app.find_game(&g1.id).unwrap();
        assert_eq!(g.name, "One");
        assert!(b.app.find_game(&g2.id).is_err());
        let view = b.app.history_view(&g).unwrap();
        assert_eq!((view.nodes.len(), view.branch.as_str()), (1, "main"));
        assert!(b.app.fsck(false).unwrap().ok());
        // 另一台电脑上的绝对路径在这里也存在（同一台测试机），可直接使用。
        assert!(b.app.locations(&g.id).unwrap().iter().all(|l| l.confirmed));
        assert!(!b.app.game_row(&g, false).unwrap().installed);
        // 磁盘上已有相同的存档（同一台测试机）：已是最新。
        assert_eq!(
            b.app.game_row(&g, false).unwrap().state,
            crate::GameState::UpToDate
        );

        // 全部导入：One 已存在（相同），Two 新建。
        let items = b.app.import_archive(&lock, &out, &[], false).unwrap();
        assert!(items.iter().all(|i| i.error.is_none()), "{items:?}");
        assert!(b.app.find_game(&g2.id).is_ok());
        drop(b.dir);
    }

    /// 在游戏库中导入别人分享的存档文件：本地没有这个游戏时新建并放在 main 上。
    #[test]
    fn import_files_auto_creates_game_from_manifest() {
        let mut a = Env::new();
        a.write("slot1/a.sav", b"one");
        let g = a.add_game("Shared");
        backup(&mut a, &g, "x");
        let zip = a.dir.path().join("share.zip");
        a.app
            .export_files(&g, Some("HEAD"), &zip, ExportFormat::Zip)
            .unwrap();

        let mut b = Env::new();
        let src = b.app.inspect_files_source(&zip).unwrap();
        assert_eq!(
            (src.game_id.as_deref(), src.manifest.game_name.as_str()),
            (None, "Shared")
        );
        let lock = b.app.repo.lock().unwrap();
        let r = b.app.import_files_auto(&lock, &zip, None, false).unwrap();
        assert!(r.created_game && r.report.snapshot.is_some());
        assert_eq!(r.report.branch.as_deref(), Some("main"));
        let gb = b.app.find_game(&g.id).unwrap();
        assert_eq!(b.app.history_view(&gb).unwrap().nodes.len(), 1);
        assert!(b.app.fsck(false).unwrap().ok());
        // 再导入一次：内容相同，不重复生成。
        let r = b.app.import_files_auto(&lock, &zip, None, false).unwrap();
        assert!(!r.created_game && r.report.preview.identical.is_some());

        // 没有导出说明的文件夹：无法判断游戏。
        let plain = b.dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("a.sav"), b"x").unwrap();
        assert!(b.app.inspect_files_source(&plain).is_err());
    }

    /// 导入的游戏在这台电脑上找不到存档位置：不显示“已删除”，也不会备份出空快照。
    #[test]
    fn imported_game_without_local_saves_stays_up_to_date() {
        let mut a = Env::new();
        a.write("a.sav", b"v1");
        let g = a.add_game("Gone");
        backup(&mut a, &g, "x");
        let out = tempfile::tempdir().unwrap();
        let file = out.path().join("g.bonfire");
        a.app
            .export_bundle(&g, &BundleScope::All, &file, false)
            .unwrap();
        drop(a); // 原电脑上的存档目录不复存在

        let mut b = Env::new();
        let lock = b.app.repo.lock().unwrap();
        let r = b.app.import_bundle(&lock, &file, None, false).unwrap();
        assert!(r.created_game);
        let gb = b.app.find_game(&g.id).unwrap();
        let row = b.app.game_row(&gb, false).unwrap();
        assert_eq!(row.state, crate::GameState::UpToDate, "{row:?}");
        assert!(row.unresolved >= 1, "{row:?}");
        let res = b
            .app
            .backup(&lock, &gb, &crate::BackupOptions::default())
            .unwrap();
        assert!(matches!(res, crate::BackupResult::NoChanges), "{res:?}");
        assert_eq!(b.app.history_view(&gb).unwrap().nodes.len(), 1);
    }

    /// 备份时，本机不能确定路径的位置沿用上一个快照中的文件。
    #[test]
    fn backup_keeps_files_of_unresolved_locations() {
        let mut env = Env::new();
        env.write("a.sav", b"v1");
        let g = env.add_game("Game");
        backup(&mut env, &g, "x");
        // 第二个位置（cfg.ini）在“这台电脑”上不能确定。
        let mut locs = env.app.locations(&g.id).unwrap();
        locs[1].resolved_path = String::new();
        locs[1].confirmed = false;
        env.app.index.set_locations(&g.id, &locs).unwrap();
        env.write("a.sav", b"v2");
        let id = backup(&mut env, &g, "y");
        let paths: Vec<String> = env
            .app
            .snapshot_files(&g, &id.to_string())
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        assert_eq!(paths, ["0/a.sav", "1/cfg.ini"]);
        assert_eq!(
            env.app.game_row(&g, false).unwrap().state,
            crate::GameState::UpToDate
        );
    }

    #[test]
    fn adopted_game_leaves_install_dependent_locations_unresolved() {
        let mut env = Env::new();
        let g = env
            .app
            .adopt_game(
                "steam-123",
                "Remote Game",
                &[
                    ("<base>/save".into(), LocationKind::Glob, FileTag::Save),
                    (
                        "<winAppData>/Remote/*.sav".into(),
                        LocationKind::Glob,
                        FileTag::Save,
                    ),
                ],
            )
            .unwrap();
        assert_eq!(
            (g.platform.as_str(), g.app_id.as_deref()),
            ("steam", Some("123"))
        );
        let locs = env.app.locations(&g.id).unwrap();
        assert!(locs[0].resolved_path.is_empty() && !locs[0].confirmed);
        assert!(locs[1].confirmed && locs[1].resolved_path.ends_with("Remote/*.sav"));
        let err = env.app.disk_path(&g.id, "0/a.sav").unwrap_err();
        assert!(err.to_string().contains("未安装"), "{err}");
        assert!(env.app.disk_path(&g.id, "1/a.sav").is_ok());
        let row = env.app.game_row(&g, false).unwrap();
        assert_eq!(
            (row.installed, row.unresolved, row.unconfirmed),
            (false, 1, 0)
        );
    }

    /// 勾选“包含截图”时，截图相册随历史包导出，导入后加入本地相册。
    #[test]
    fn bundle_can_carry_the_screenshot_album() {
        let mut a = Env::new();
        a.write("a.sav", b"v1");
        let g = a.add_game("Shots");
        backup(&mut a, &g, "x");
        let shots = a.dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(shots.join("one.png"), png.get_ref()).unwrap();
        a.app.add_shot_dir(&g, &shots).unwrap();
        let lock = a.app.repo.lock().unwrap();
        assert_eq!(a.app.sync_shots(&lock, &g).unwrap().added, 1);
        drop(lock);
        let out = tempfile::tempdir().unwrap();
        let without = out.path().join("no.bonfire");
        let with = out.path().join("yes.bonfire");
        assert_eq!(
            a.app
                .export_bundle(&g, &BundleScope::All, &without, false)
                .unwrap()
                .shots,
            0
        );
        assert_eq!(
            a.app
                .export_bundle(&g, &BundleScope::All, &with, true)
                .unwrap()
                .shots,
            1
        );

        let mut b = Env::new();
        assert_eq!(b.app.inspect_bundle(&without, None).unwrap().shots, 0);
        let p = b.app.inspect_bundle(&with, None).unwrap();
        assert_eq!((p.shots, p.new_shots), (1, 1));
        let lock = b.app.repo.lock().unwrap();
        let r = b.app.import_bundle(&lock, &with, None, false).unwrap();
        assert_eq!(r.imported_shots, 1);
        let gb = b.app.find_game(&g.id).unwrap();
        let album = b.app.album(&gb).unwrap();
        assert_eq!(album.shots.len(), 1);
        // 原图原样放在包中，导入后存为普通文件。
        assert_eq!(album.shots[0].file, "one.png");
        assert_eq!(
            std::fs::read(album.dir.join("one.png")).unwrap(),
            png.get_ref().as_slice()
        );
        assert!(
            b.app
                .shot_thumb(&gb.id, &album.shots[0].blob)
                .unwrap()
                .is_file()
        );
        // 回收站中的截图不导出。
        let lock_a = a.app.repo.lock().unwrap();
        let blob = a.app.album(&g).unwrap().shots[0].blob;
        a.app.delete_shots(&lock_a, &g, &[blob]).unwrap();
        drop(lock_a);
        let trashed = out.path().join("trash.bonfire");
        assert_eq!(
            a.app
                .export_bundle(&g, &BundleScope::All, &trashed, true)
                .unwrap()
                .shots,
            0
        );
        // 再导入一次：相册中已有。
        assert_eq!(b.app.inspect_bundle(&with, None).unwrap().new_shots, 0);
        // 重建索引后仍在。
        b.app.reindex(&lock).unwrap();
        assert_eq!(b.app.album(&gb).unwrap().shots.len(), 1);
        drop(a);
    }

    #[test]
    fn folders_are_unique_and_safe() {
        let loc = |p: &str, kind| LocationRecord {
            path_template: p.into(),
            resolved_path: p.into(),
            kind,
            tag: Default::default(),
            source: "user".into(),
            confirmed: true,
        };
        let f = location_folders(&[
            loc("C:/a/Save", LocationKind::Dir),
            loc("D:/b/save", LocationKind::Dir),
            loc("C:/x/y/*.sav", LocationKind::Glob),
            loc("HKCU\\Software\\X", LocationKind::Registry),
        ]);
        assert_eq!(f, ["Save", "save-1", "y", "registry"]);
    }
}
