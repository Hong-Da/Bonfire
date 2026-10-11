# HIS 历史、分支、标签与备注

时间线、分支、标签、备注修改与搜索、删除快照。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| HIS-01 | 历史与搜索备注 | 按分支查看、按备注搜索 | `crates/bonfire-app/src/history.rs` 测试；CLI 用例 `log-ok`、`log-grep`、`log-unknown-game` | 0.1.0 | 有效 |
| HIS-02 | 修改备注 | 保留修改历史 | `crates/bonfire-core/src/refs.rs` 测试；CLI 用例 `comment-ok`、`comment-missing-message` | 0.1.0 | 有效 |
| HIS-03 | 分支与标签 | 新建、改名、删除、本机主存档 | `crates/bonfire-core/src/refs.rs`、`crates/bonfire-app/src/primary.rs`、`crates/bonfire-app/src/prune.rs` 测试；CLI 用例 `tag-ok`、`tag-duplicate`、`branch-list`、`branch-no-snapshots`、`branch-rename`、`branch-rename-exists`、`branch-primary-set`、`branch-primary-show`、`branch-primary-delete-refused`、`branch-delete`、`branch-delete-missing`、`tag-delete` | 0.1.0 | 有效 |
| HIS-04 | 删除快照 | 后续快照改接；带标签与受存档安全保护的不能删 | `crates/bonfire-app/src/prune.rs` 测试；CLI 用例 `rm-snapshot`、`rm-snapshot-dry-run`、`rm-snapshot-protected`、`rm-tagged-snapshot` | 0.1.0 | 有效 |
| HIS-05 | 多选删除快照 | Ctrl / Shift 多选，删除前检查与路线变化提示 | `crates/bonfire-app/src/bulk_delete.rs` 测试；CLI 用例 `rm-many-preview`；`app/src/u10.test.tsx`（多选删除快照） | 0.1.0 | 有效 |
