//! 引用：分支、标签、HEAD、reflog 与备注注记（文档 6.1、6.3、6.4）。
//!
//! ```text
//! refs/<游戏>/heads/<分支>    分支指针（文件内容为快照 ID）
//! refs/<游戏>/tags/<标签>     标签，创建后不可移动
//! refs/<游戏>/HEAD            当前分支，以及“已从某快照恢复”的标记（JSON）
//! logs/<游戏>.jsonl           引用变更日志（reflog），每行一条
//! notes/<快照>.json           事后修改的备注及修改历史
//! ```
//!
//! 所有写操作都需要持有 [`RepoLock`]。

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoContext, Result};
use crate::object::{Kind, Snapshot};
use crate::store::write_atomic;
use crate::{ObjectId, Repo, RepoLock};

pub const DEFAULT_BRANCH: &str = "main";
/// 用哈希前缀引用快照时的最少位数。
pub const MIN_PREFIX: usize = 6;
const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefKind {
    Branch,
    Tag,
}

impl RefKind {
    fn dir(self) -> &'static str {
        match self {
            RefKind::Branch => "heads",
            RefKind::Tag => "tags",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefName {
    pub kind: RefKind,
    pub name: String,
}

impl RefName {
    pub fn branch(name: &str) -> Self {
        Self {
            kind: RefKind::Branch,
            name: name.to_string(),
        }
    }

    pub fn tag(name: &str) -> Self {
        Self {
            kind: RefKind::Tag,
            name: name.to_string(),
        }
    }
}

impl std::fmt::Display for RefName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.kind.dir(), self.name)
    }
}

/// 游戏 ID：`<平台>-<ID>`，只允许 ASCII 字母、数字、`.`、`_`、`-`。
pub fn validate_game_id(id: &str) -> Result<()> {
    let bad = |reason| {
        Err(CoreError::InvalidName {
            name: id.to_string(),
            reason,
        })
    };
    if id.is_empty() || id.len() > 128 {
        return bad("长度应为 1–128");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || id.starts_with('.')
    {
        return bad("只能包含字母、数字、. _ -");
    }
    Ok(())
}

/// 分支名与标签名：允许中文等 Unicode，但必须能作为 Windows 文件名，且不与引用语法冲突。
pub fn validate_ref_name(name: &str) -> Result<()> {
    let bad = |reason| {
        Err(CoreError::InvalidName {
            name: name.to_string(),
            reason,
        })
    };
    let count = name.chars().count();
    if count == 0 || count > MAX_NAME_CHARS {
        return bad("长度应为 1–100 个字符");
    }
    if name.trim() != name || name.ends_with('.') || name.starts_with('.') {
        return bad("不能以空格或 . 开头或结尾");
    }
    if name
        .chars()
        .any(|c| c.is_control() || "<>:\"/\\|?*~^".contains(c))
    {
        return bad("不能包含 < > : \" / \\ | ? * ~ ^ 或控制字符");
    }
    if name.eq_ignore_ascii_case("HEAD") {
        return bad("HEAD 是保留名");
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        return bad("是 Windows 保留的设备名");
    }
    Ok(())
}

/// HEAD：当前分支；从旧快照恢复后 `detached` 记录当前存档对应的快照，
/// 下次备份以它为父快照，并询问“创建新分支 / 移动当前分支”（文档 6.4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<ObjectId>,
}

impl Default for Head {
    fn default() -> Self {
        Self {
            branch: DEFAULT_BRANCH.to_string(),
            detached: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflogEntry {
    pub time: jiff::Timestamp,
    /// 如 `heads/main`、`tags/before-boss`、`HEAD`。
    #[serde(rename = "ref")]
    pub reference: String,
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
    /// 如 `backup`、`restore`、`branch`、`tag`、`delete`。
    pub action: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEdit {
    pub time: jiff::Timestamp,
    /// 被替换掉的备注。
    pub previous: String,
}

/// 事后修改的备注：显示时优先于快照对象中的原始备注。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub comment: String,
    pub history: Vec<NoteEdit>,
}

impl Repo {
    fn game_dir(&self, game: &str) -> PathBuf {
        self.root().join("refs").join(game)
    }

    fn ref_path(&self, game: &str, r: &RefName) -> PathBuf {
        self.game_dir(game).join(r.kind.dir()).join(&r.name)
    }

    fn reflog_path(&self, game: &str) -> PathBuf {
        self.root().join("logs").join(format!("{game}.jsonl"))
    }

    fn note_path(&self, snapshot: &ObjectId) -> PathBuf {
        self.root()
            .join("notes")
            .join(format!("{}.json", snapshot.to_hex()))
    }

    /// 游戏的别名：从其他电脑导入的历史中，快照记录的游戏 ID（如自定义游戏在另一台电脑上的 ID）。
    pub fn aliases(&self, game: &str) -> Result<Vec<String>> {
        let path = self.game_dir(game).join("aliases");
        match std::fs::read_to_string(&path) {
            Ok(t) => Ok(t
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    pub fn add_alias(&self, _lock: &RepoLock, game: &str, alias: &str) -> Result<()> {
        validate_game_id(game)?;
        validate_game_id(alias)?;
        let mut all = self.aliases(game)?;
        if all.iter().any(|a| a == alias) {
            return Ok(());
        }
        all.push(alias.to_string());
        let dir = self.game_dir(game);
        std::fs::create_dir_all(&dir).at(&dir)?;
        write_atomic(
            &self.tmp_dir(),
            &dir.join("aliases"),
            format!(
                "{}
",
                all.join(
                    "
"
                )
            )
            .as_bytes(),
            false,
        )
    }

    /// 有引用记录的游戏。
    pub fn games(&self) -> Result<Vec<String>> {
        let dir = self.root().join("refs");
        let mut out: Vec<String> = std::fs::read_dir(&dir)
            .at(&dir)?
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        Ok(out)
    }

    pub fn read_ref(&self, game: &str, r: &RefName) -> Result<Option<ObjectId>> {
        let path = self.ref_path(game, r);
        match std::fs::read_to_string(&path) {
            Ok(t) => t
                .trim()
                .parse()
                .map(Some)
                .map_err(|e: crate::id::ParseIdError| CoreError::BadFile {
                    path,
                    reason: e.to_string(),
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    /// 某类引用的名称（按名称排序）。
    fn ref_names(&self, game: &str, kind: RefKind) -> Result<Vec<String>> {
        let dir = self.game_dir(game).join(kind.dir());
        let mut names: Vec<String> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(CoreError::io(dir, e)),
        };
        names.sort();
        Ok(names)
    }

    /// 游戏的全部分支与标签。
    pub fn list_refs(&self, game: &str) -> Result<Vec<(RefName, ObjectId)>> {
        let mut out = Vec::new();
        for kind in [RefKind::Branch, RefKind::Tag] {
            for name in self.ref_names(game, kind)? {
                let r = RefName { kind, name };
                if let Some(id) = self.read_ref(game, &r)? {
                    out.push((r, id));
                }
            }
        }
        Ok(out)
    }

    /// 同类引用中是否已有同名（不区分大小写，与 Windows 文件系统一致）的引用。
    fn find_existing(&self, game: &str, r: &RefName) -> Result<Option<String>> {
        let lower = r.name.to_lowercase();
        Ok(self
            .ref_names(game, r.kind)?
            .into_iter()
            .find(|n| n.to_lowercase() == lower))
    }

    /// 比较并更新引用：当前值必须等于 `expected_old`（`None` 表示引用尚不存在），否则返回冲突。
    /// 标签一经创建不可移动。
    #[allow(clippy::too_many_arguments)]
    pub fn update_ref(
        &self,
        _lock: &RepoLock,
        game: &str,
        r: &RefName,
        expected_old: Option<ObjectId>,
        new: ObjectId,
        action: &str,
        message: &str,
    ) -> Result<()> {
        validate_game_id(game)?;
        validate_ref_name(&r.name)?;
        let current = self.read_ref(game, r)?;
        if current.is_none()
            && let Some(other) = self.find_existing(game, r)?
            && other != r.name
        {
            return Err(CoreError::RefExists(format!("{}/{other}", r.kind.dir())));
        }
        if r.kind == RefKind::Tag && current.is_some() {
            return Err(CoreError::RefExists(r.to_string()));
        }
        if current != expected_old {
            return Err(CoreError::RefConflict(r.to_string()));
        }
        let path = self.ref_path(game, r);
        let dir = path.parent().expect("引用路径有上级目录");
        std::fs::create_dir_all(dir).at(dir)?;
        write_atomic(&self.tmp_dir(), &path, format!("{new}\n").as_bytes(), false)?;
        self.append_reflog(game, &r.to_string(), current, Some(new), action, message)
    }

    /// 创建分支（已存在时报错）。
    pub fn create_branch(
        &self,
        lock: &RepoLock,
        game: &str,
        name: &str,
        target: ObjectId,
    ) -> Result<()> {
        let r = RefName::branch(name);
        validate_ref_name(name)?;
        if let Some(existing) = self.find_existing(game, &r)? {
            return Err(CoreError::RefExists(format!("heads/{existing}")));
        }
        self.update_ref(
            lock,
            game,
            &r,
            None,
            target,
            "branch",
            &format!("创建分支 {name}"),
        )
    }

    pub fn create_tag(
        &self,
        lock: &RepoLock,
        game: &str,
        name: &str,
        target: ObjectId,
    ) -> Result<()> {
        let r = RefName::tag(name);
        validate_ref_name(name)?;
        if let Some(existing) = self.find_existing(game, &r)? {
            return Err(CoreError::RefExists(format!("tags/{existing}")));
        }
        self.update_ref(
            lock,
            game,
            &r,
            None,
            target,
            "tag",
            &format!("创建标签 {name}"),
        )
    }

    /// 删除引用。快照对象本身保留，由 reflog 记录，GC 时按保留期限清理。
    pub fn delete_ref(&self, _lock: &RepoLock, game: &str, r: &RefName) -> Result<()> {
        let old = self
            .read_ref(game, r)?
            .ok_or_else(|| CoreError::RefNotFound(r.to_string()))?;
        let path = self.ref_path(game, r);
        std::fs::remove_file(&path).at(&path)?;
        self.append_reflog(
            game,
            &r.to_string(),
            Some(old),
            None,
            "delete",
            &format!("删除 {r}"),
        )
    }

    pub fn head(&self, game: &str) -> Result<Head> {
        let path = self.game_dir(game).join("HEAD");
        match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| CoreError::BadFile {
                path,
                reason: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Head::default()),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    pub fn set_head(&self, _lock: &RepoLock, game: &str, head: &Head) -> Result<()> {
        validate_game_id(game)?;
        validate_ref_name(&head.branch)?;
        let old = self.head(game)?;
        let dir = self.game_dir(game);
        std::fs::create_dir_all(&dir).at(&dir)?;
        let json = serde_json::to_vec(head).expect("HEAD 序列化不会失败");
        write_atomic(&self.tmp_dir(), &dir.join("HEAD"), &json, false)?;
        if old.detached != head.detached || old.branch != head.branch {
            let msg = match head.detached {
                Some(id) => format!("{} 已从 {} 恢复", head.branch, id.short(12)),
                None => format!("切换到 {}", head.branch),
            };
            self.append_reflog(game, "HEAD", old.detached, head.detached, "head", &msg)?;
        }
        Ok(())
    }

    fn append_reflog(
        &self,
        game: &str,
        reference: &str,
        old: Option<ObjectId>,
        new: Option<ObjectId>,
        action: &str,
        message: &str,
    ) -> Result<()> {
        let entry = ReflogEntry {
            time: jiff::Timestamp::now(),
            reference: reference.to_string(),
            old,
            new,
            action: action.to_string(),
            message: message.to_string(),
        };
        let mut line = serde_json::to_vec(&entry).expect("reflog 序列化不会失败");
        line.push(b'\n');
        let path = self.reflog_path(game);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .at(&path)?;
        f.write_all(&line).at(&path)?;
        f.sync_all().at(&path)
    }

    /// 游戏的 reflog（时间顺序）。崩溃留下的不完整行会被跳过。
    pub fn reflog(&self, game: &str) -> Result<Vec<ReflogEntry>> {
        let path = self.reflog_path(game);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(CoreError::io(path, e)),
        };
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str(l) {
                Ok(e) => Some(e),
                Err(err) => {
                    tracing::warn!(path = %path.display(), "跳过无法解析的 reflog 行：{err}");
                    None
                }
            })
            .collect())
    }

    pub fn note(&self, snapshot: &ObjectId) -> Result<Option<Note>> {
        let path = self.note_path(snapshot);
        match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t)
                .map(Some)
                .map_err(|e| CoreError::BadFile {
                    path,
                    reason: e.to_string(),
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    /// 修改快照备注：不重写快照对象，写入注记并保留修改历史（文档 6.3）。
    pub fn set_note(&self, _lock: &RepoLock, snapshot: &ObjectId, comment: &str) -> Result<Note> {
        let snap = self.get_snapshot(snapshot)?;
        let mut note = self.note(snapshot)?.unwrap_or(Note {
            comment: snap.comment.clone(),
            history: Vec::new(),
        });
        if note.comment == comment {
            return Ok(note);
        }
        note.history.push(NoteEdit {
            time: jiff::Timestamp::now(),
            previous: std::mem::take(&mut note.comment),
        });
        note.comment = comment.to_string();
        let json = serde_json::to_vec_pretty(&note).expect("注记序列化不会失败");
        write_atomic(&self.tmp_dir(), &self.note_path(snapshot), &json, false)?;
        Ok(note)
    }

    /// 显示用的备注：有注记时取注记，否则取快照中的原始备注。
    pub fn effective_comment(&self, id: &ObjectId, snap: &Snapshot) -> Result<String> {
        Ok(self
            .note(id)?
            .map(|n| n.comment)
            .unwrap_or_else(|| snap.comment.clone()))
    }

    /// 当前状态对应的快照：已从旧快照恢复时为该快照，否则为当前分支的最新快照。
    pub fn head_snapshot(&self, game: &str) -> Result<Option<ObjectId>> {
        let head = self.head(game)?;
        match head.detached {
            Some(id) => Ok(Some(id)),
            None => self.read_ref(game, &RefName::branch(&head.branch)),
        }
    }

    /// 解析快照引用：`HEAD`、分支名、标签名、`heads/<名>`、`tags/<名>`、
    /// 哈希前缀（≥ 6 位，可带 `b3:`），以及在上述任一形式后加 `~n` 表示沿第一父快照回退 n 步。
    pub fn resolve(&self, game: &str, spec: &str) -> Result<ObjectId> {
        validate_game_id(game)?;
        let spec = spec.trim();
        let (base, steps) = match spec.rfind('~') {
            Some(i) => {
                let n = &spec[i + 1..];
                let steps = if n.is_empty() {
                    1
                } else {
                    n.parse::<usize>().map_err(|_| CoreError::InvalidName {
                        name: spec.to_string(),
                        reason: "~ 后应为数字",
                    })?
                };
                (&spec[..i], steps)
            }
            None => (spec, 0),
        };
        let mut id = self.resolve_base(game, base)?;
        for _ in 0..steps {
            let snap = self.get_snapshot(&id)?;
            id = *snap
                .parents
                .first()
                .ok_or_else(|| CoreError::NoParent(spec.to_string()))?;
        }
        Ok(id)
    }

    fn resolve_base(&self, game: &str, base: &str) -> Result<ObjectId> {
        if base.is_empty() {
            return Err(CoreError::InvalidName {
                name: base.to_string(),
                reason: "引用不能为空",
            });
        }
        if base == "HEAD" {
            return self
                .head_snapshot(game)?
                .ok_or_else(|| CoreError::NoSnapshots(game.to_string()));
        }
        for (prefix, kind) in [("heads/", RefKind::Branch), ("tags/", RefKind::Tag)] {
            if let Some(name) = base.strip_prefix(prefix) {
                let r = RefName {
                    kind,
                    name: name.to_string(),
                };
                return self
                    .read_ref(game, &r)?
                    .ok_or_else(|| CoreError::RefNotFound(base.to_string()));
            }
        }
        let branch = self.read_ref(game, &RefName::branch(base))?;
        let tag = self.read_ref(game, &RefName::tag(base))?;
        match (branch, tag) {
            (Some(_), Some(_)) => {
                return Err(CoreError::AmbiguousRef {
                    name: base.to_string(),
                    candidates: format!("heads/{base}、tags/{base}"),
                });
            }
            (Some(id), None) | (None, Some(id)) => return Ok(id),
            (None, None) => {}
        }
        let hex = base
            .strip_prefix(crate::id::PREFIX)
            .unwrap_or(base)
            .to_ascii_lowercase();
        if !hex.is_empty() && hex.len() <= 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if hex.len() < MIN_PREFIX {
                return Err(CoreError::InvalidName {
                    name: base.to_string(),
                    reason: "哈希前缀至少 6 位",
                });
            }
            return self.find_snapshot_by_prefix(game, &hex, base);
        }
        Err(CoreError::RefNotFound(base.to_string()))
    }

    /// 按哈希前缀查找属于该游戏的快照。
    fn find_snapshot_by_prefix(&self, game: &str, hex: &str, spec: &str) -> Result<ObjectId> {
        let dir = self
            .objects_dir()
            .join(Kind::Snapshot.dir())
            .join(&hex[..2]);
        let rest = &hex[2..];
        let mut found = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(rest) {
                    let id: ObjectId =
                        format!("{}{name}", &hex[..2])
                            .parse()
                            .map_err(|_| CoreError::BadFile {
                                path: e.path(),
                                reason: "文件名不是对象 ID".into(),
                            })?;
                    if self.get_snapshot(&id)?.game == game {
                        found.push(id);
                    }
                }
            }
        }
        match found.len() {
            0 => Err(CoreError::RefNotFound(spec.to_string())),
            1 => Ok(found[0]),
            _ => {
                found.sort();
                let candidates = found
                    .iter()
                    .map(|id| id.short(12))
                    .collect::<Vec<_>>()
                    .join("、");
                Err(CoreError::AmbiguousRef {
                    name: spec.to_string(),
                    candidates,
                })
            }
        }
    }

    /// 沿第一父快照向前遍历，最多 `limit` 个（含起点）。
    pub fn first_parent_chain(
        &self,
        start: ObjectId,
        limit: usize,
    ) -> Result<Vec<(ObjectId, Snapshot)>> {
        let mut out = Vec::new();
        let mut cur = Some(start);
        while let Some(id) = cur {
            if out.len() >= limit {
                break;
            }
            let snap = self.get_snapshot(&id)?;
            cur = snap.parents.first().copied();
            out.push((id, snap));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{SnapshotStats, Tree, Trigger};

    const GAME: &str = "custom-test";

    fn repo() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        let r = Repo::init(dir.path()).unwrap();
        (dir, r)
    }

    fn snap(r: &Repo, game: &str, parents: Vec<ObjectId>, comment: &str) -> ObjectId {
        let tree = r.put_tree(&Tree::new(vec![])).unwrap();
        r.put_snapshot(&Snapshot {
            game: game.into(),
            tree,
            parents,
            created_at: "2026-10-09T12:00:00+08:00".into(),
            trigger: Trigger::Manual,
            host: "PC".into(),
            comment: comment.into(),
            stats: SnapshotStats::default(),
            playtime_min: None,
        })
        .unwrap()
    }

    /// main: a ← b ← c
    fn linear(r: &Repo, lock: &RepoLock) -> [ObjectId; 3] {
        let a = snap(r, GAME, vec![], "a");
        let b = snap(r, GAME, vec![a], "b");
        let c = snap(r, GAME, vec![b], "c");
        let main = RefName::branch("main");
        r.update_ref(lock, GAME, &main, None, a, "backup", "")
            .unwrap();
        r.update_ref(lock, GAME, &main, Some(a), b, "backup", "")
            .unwrap();
        r.update_ref(lock, GAME, &main, Some(b), c, "backup", "")
            .unwrap();
        [a, b, c]
    }

    #[test]
    fn name_validation() {
        for ok in [
            "main",
            "ng-plus",
            "二周目",
            "Boss 前",
            "before.boss",
            "v1_2",
        ] {
            validate_ref_name(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            "", " main", "main ", ".x", "x.", "a/b", "a\\b", "a:b", "a~1", "a^", "HEAD", "head",
            "CON", "nul.txt", "COM1", "a*b", "a\tb",
        ] {
            assert!(validate_ref_name(bad).is_err(), "{bad:?} 应被拒绝");
        }
        assert!(validate_ref_name(&"长".repeat(101)).is_err());
        validate_game_id("steam-1245620").unwrap();
        validate_game_id("custom-4f9c2a1e-8b7d").unwrap();
        assert!(validate_game_id("../evil").is_err());
        assert!(validate_game_id("游戏").is_err());
    }

    #[test]
    fn update_ref_compare_and_swap() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, c] = linear(&r, &lock);
        let main = RefName::branch("main");
        assert_eq!(r.read_ref(GAME, &main).unwrap(), Some(c));
        // 旧值不符：拒绝。
        assert!(matches!(
            r.update_ref(&lock, GAME, &main, Some(a), b, "backup", ""),
            Err(CoreError::RefConflict(_))
        ));
        assert!(matches!(
            r.update_ref(&lock, GAME, &main, None, b, "backup", ""),
            Err(CoreError::RefConflict(_))
        ));
        assert_eq!(r.read_ref(GAME, &main).unwrap(), Some(c));
        assert_eq!(r.games().unwrap(), vec![GAME.to_string()]);
    }

    #[test]
    fn tags_are_immutable_and_names_case_insensitive() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, _] = linear(&r, &lock);
        r.create_tag(&lock, GAME, "Boss 前", a).unwrap();
        assert!(matches!(
            r.create_tag(&lock, GAME, "Boss 前", b),
            Err(CoreError::RefExists(_))
        ));
        assert!(matches!(
            r.create_tag(&lock, GAME, "boss 前", b),
            Err(CoreError::RefExists(_))
        ));
        let tag = RefName::tag("Boss 前");
        assert!(matches!(
            r.update_ref(&lock, GAME, &tag, Some(a), b, "tag", ""),
            Err(CoreError::RefExists(_))
        ));
        assert!(matches!(
            r.create_branch(&lock, GAME, "MAIN", b),
            Err(CoreError::RefExists(_))
        ));

        r.create_branch(&lock, GAME, "二周目", b).unwrap();
        let refs: Vec<String> = r
            .list_refs(GAME)
            .unwrap()
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();
        assert_eq!(refs, ["heads/main", "heads/二周目", "tags/Boss 前"]);

        r.delete_ref(&lock, GAME, &tag).unwrap();
        assert_eq!(r.read_ref(GAME, &tag).unwrap(), None);
        assert!(matches!(
            r.delete_ref(&lock, GAME, &tag),
            Err(CoreError::RefNotFound(_))
        ));
    }

    #[test]
    fn reflog_records_changes() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, c] = linear(&r, &lock);
        r.create_tag(&lock, GAME, "t", a).unwrap();
        let log = r.reflog(GAME).unwrap();
        assert_eq!(log.len(), 4);
        assert_eq!((log[0].old, log[0].new), (None, Some(a)));
        assert_eq!((log[2].old, log[2].new), (Some(b), Some(c)));
        assert_eq!(log[3].reference, "tags/t");
        assert_eq!(log[3].action, "tag");
    }

    #[test]
    fn reflog_skips_torn_last_line() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        linear(&r, &lock);
        let p = r.reflog_path(GAME);
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(b"{\"time\":\"2026-").unwrap();
        assert_eq!(r.reflog(GAME).unwrap().len(), 3);
    }

    #[test]
    fn resolve_forms() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, c] = linear(&r, &lock);
        r.create_tag(&lock, GAME, "before-boss", b).unwrap();
        assert_eq!(r.resolve(GAME, "HEAD").unwrap(), c);
        assert_eq!(r.resolve(GAME, "HEAD~").unwrap(), b);
        assert_eq!(r.resolve(GAME, "HEAD~2").unwrap(), a);
        assert_eq!(r.resolve(GAME, "main~1").unwrap(), b);
        assert_eq!(r.resolve(GAME, "before-boss").unwrap(), b);
        assert_eq!(r.resolve(GAME, "tags/before-boss~1").unwrap(), a);
        assert_eq!(r.resolve(GAME, "heads/main").unwrap(), c);
        assert_eq!(r.resolve(GAME, &a.short(8)).unwrap(), a);
        assert_eq!(r.resolve(GAME, &format!("b3:{}", b.short(10))).unwrap(), b);
        assert_eq!(r.resolve(GAME, &c.to_hex().to_uppercase()).unwrap(), c);
        assert!(matches!(
            r.resolve(GAME, "HEAD~3"),
            Err(CoreError::NoParent(_))
        ));
        assert!(matches!(
            r.resolve(GAME, &a.short(5)),
            Err(CoreError::InvalidName { .. })
        ));
        assert!(matches!(
            r.resolve(GAME, "nope"),
            Err(CoreError::RefNotFound(_))
        ));
        assert!(matches!(
            r.resolve(GAME, "HEAD~x"),
            Err(CoreError::InvalidName { .. })
        ));
    }

    #[test]
    fn resolve_ambiguous_name() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, _] = linear(&r, &lock);
        r.create_branch(&lock, GAME, "x", a).unwrap();
        r.create_tag(&lock, GAME, "x", b).unwrap();
        assert!(matches!(
            r.resolve(GAME, "x"),
            Err(CoreError::AmbiguousRef { .. })
        ));
        assert_eq!(r.resolve(GAME, "tags/x").unwrap(), b);
    }

    #[test]
    fn prefix_only_matches_same_game() {
        let (_d, r) = repo();
        let other = snap(&r, "custom-other", vec![], "o");
        assert!(matches!(
            r.resolve(GAME, &other.short(10)),
            Err(CoreError::RefNotFound(_))
        ));
        assert_eq!(r.resolve("custom-other", &other.short(10)).unwrap(), other);
    }

    #[test]
    fn head_default_detached_and_empty_game() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        assert_eq!(r.head(GAME).unwrap(), Head::default());
        assert!(matches!(
            r.resolve(GAME, "HEAD"),
            Err(CoreError::NoSnapshots(_))
        ));
        let [a, _, c] = linear(&r, &lock);
        r.set_head(
            &lock,
            GAME,
            &Head {
                branch: "main".into(),
                detached: Some(a),
            },
        )
        .unwrap();
        assert_eq!(
            r.resolve(GAME, "HEAD").unwrap(),
            a,
            "恢复后 HEAD 指向被恢复的快照"
        );
        assert_eq!(r.resolve(GAME, "main").unwrap(), c, "分支本身不移动");
        assert_eq!(r.reflog(GAME).unwrap().last().unwrap().action, "head");
        r.set_head(&lock, GAME, &Head::default()).unwrap();
        assert_eq!(r.resolve(GAME, "HEAD").unwrap(), c);
    }

    #[test]
    fn notes_override_comment_and_keep_history() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let id = snap(&r, GAME, vec![], "打完大树守卫");
        let s = r.get_snapshot(&id).unwrap();
        assert_eq!(r.effective_comment(&id, &s).unwrap(), "打完大树守卫");
        r.set_note(&lock, &id, "打完大树守卫，强化 +5 前").unwrap();
        let n = r.set_note(&lock, &id, "第三版").unwrap();
        assert_eq!(n.comment, "第三版");
        let prev: Vec<_> = n.history.iter().map(|h| h.previous.as_str()).collect();
        assert_eq!(prev, ["打完大树守卫", "打完大树守卫，强化 +5 前"]);
        assert_eq!(r.effective_comment(&id, &s).unwrap(), "第三版");
        // 快照对象未改变，ID 不变。
        assert_eq!(r.get_snapshot(&id).unwrap().comment, "打完大树守卫");
        // 相同内容不新增历史。
        assert_eq!(r.set_note(&lock, &id, "第三版").unwrap().history.len(), 2);
    }

    #[test]
    fn first_parent_chain_respects_limit() {
        let (_d, r) = repo();
        let lock = r.lock().unwrap();
        let [a, b, c] = linear(&r, &lock);
        let ids: Vec<_> = r
            .first_parent_chain(c, 10)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, [c, b, a]);
        assert_eq!(r.first_parent_chain(c, 2).unwrap().len(), 2);
    }
}
