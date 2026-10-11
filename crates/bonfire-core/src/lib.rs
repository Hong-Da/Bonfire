//! 引擎·版本存储：内容寻址对象库、refs、索引与变更检测（文档 §5、§6）。
//!
//! 本 crate 不感知界面，由 `bonfire-app` 的各服务调用。

pub mod crypt;
pub mod error;
pub mod fault;
pub mod fsck;
pub mod gc;
pub mod id;
pub mod index;
pub mod object;
pub mod refs;
pub mod repo;
pub mod scan;
pub mod store;

pub use error::{CoreError, Result};
pub use id::ObjectId;
pub use index::Index;
pub use object::{Blob, FileTag, Kind, Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
pub use refs::{Head, Note, RefKind, RefName, ReflogEntry};
pub use repo::{Repo, RepoConfig, RepoLock};
pub use store::WriteStats;
