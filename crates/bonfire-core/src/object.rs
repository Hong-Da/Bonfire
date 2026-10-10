//! 对象类型（文档 6.2）。
//!
//! | 对象 | 内容 | ID |
//! |---|---|---|
//! | Chunk | 文件内容的一个分块 | BLAKE3(原始字节) |
//! | Blob | 文件大小 + 有序 Chunk 列表 | BLAKE3(整个文件内容) |
//! | Tree | 快照内所有文件 | BLAKE3(规范化 JSON) |
//! | Snapshot | Tree、父快照、时间、触发方式、备注等 | BLAKE3(规范化 JSON) |
//!
//! 小文件的 Blob ID 与其唯一 Chunk 的 ID 相同，因此各类对象分目录存放。
//! 规范化 JSON：字段顺序由结构体定义固定，Tree 条目按路径排序，不含空白。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ObjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Chunk,
    Blob,
    Tree,
    Snapshot,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Chunk, Kind::Blob, Kind::Tree, Kind::Snapshot];

    /// 在 `objects/` 下的子目录名。
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Chunk => "chunks",
            Kind::Blob => "blobs",
            Kind::Tree => "trees",
            Kind::Snapshot => "snapshots",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Chunk => "分块",
            Kind::Blob => "文件",
            Kind::Tree => "清单",
            Kind::Snapshot => "快照",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub size: u64,
    pub chunks: Vec<ObjectId>,
}

/// 文件在存档规则中的标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileTag {
    #[default]
    Save,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    /// 相对路径，以存档位置编号为前缀、`/` 分隔，如 `0/EldenRing/ER0000.sl2`。
    pub path: String,
    pub blob: ObjectId,
    pub size: u64,
    /// 备份时的修改时间（Unix 纪元起的纳秒）。
    pub mtime_ns: i64,
    pub tag: FileTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    /// 按路径排序后构造，保证同样的内容得到同样的 ID。
    pub fn new(mut entries: Vec<TreeEntry>) -> Self {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Self { entries }
    }

    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    Manual,
    AutoExit,
    AutoTimer,
    /// 存档变化后自动备份。
    AutoChange,
    PreRestore,
    /// 从别人分享或其他电脑导出的存档文件导入。
    Import,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotStats {
    pub files: u64,
    pub bytes: u64,
    pub changed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub game: String,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    /// RFC 3339 时间，含时区。
    pub created_at: String,
    pub trigger: Trigger,
    pub host: String,
    pub comment: String,
    pub stats: SnapshotStats,
    /// 快照时的游戏时长（分钟），未知时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playtime_min: Option<u64>,
}

/// 序列化为规范化 JSON 字节。
pub(crate) fn canonical_json<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).expect("对象序列化不会失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> TreeEntry {
        TreeEntry {
            path: path.into(),
            blob: ObjectId::of(path.as_bytes()),
            size: 1,
            mtime_ns: 0,
            tag: FileTag::Save,
        }
    }

    #[test]
    fn tree_is_order_independent() {
        let a = Tree::new(vec![entry("0/b"), entry("0/a"), entry("1/存档")]);
        let b = Tree::new(vec![entry("1/存档"), entry("0/a"), entry("0/b")]);
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn snapshot_json_shape_matches_doc() {
        let s = Snapshot {
            game: "steam-1245620".into(),
            tree: ObjectId::of(b"t"),
            parents: vec![],
            created_at: "2026-10-08T21:14:03+08:00".into(),
            trigger: Trigger::PreRestore,
            host: "DESKTOP-01".into(),
            comment: "打完大树守卫".into(),
            stats: SnapshotStats {
                files: 12,
                bytes: 48213504,
                changed: 2,
            },
            playtime_min: None,
        };
        let v: serde_json::Value = serde_json::from_slice(&canonical_json(&s)).unwrap();
        assert_eq!(v["createdAt"], "2026-10-08T21:14:03+08:00");
        assert_eq!(v["trigger"], "pre-restore");
        assert_eq!(v["stats"]["changed"], 2);
        assert!(v.get("playtimeMin").is_none());
    }
}
