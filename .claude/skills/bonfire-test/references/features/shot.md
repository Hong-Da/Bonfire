# SHOT 截图

截图收集、相册、回收站、写回。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| SHOT-01 | 收集截图 | Steam、游戏栏、公用与游戏自己的截图文件夹 | `crates/bonfire-discovery/src/shots.rs`、`crates/bonfire-app/src/shots.rs` 测试；CLI 用例 `shots-dir-add`、`shots-dir-add-missing`、`shots-sync`、`shots-sync-unknown-game`、`shots-found`、`shots-found-extra-arg`、`game-add-shots`、`game-add-shots-missing-dir` | 0.1.0 | 有效 |
| SHOT-02 | 相册与导出 | 列出、导出截图 | CLI 用例 `shots-list`、`shots-list-unknown-game`、`shots-export`、`shots-export-empty`；`app/src/shots.test.tsx` | 0.1.0 | 有效 |
| SHOT-03 | 回收站与删除 | 移到回收站、恢复、彻底删除 | `crates/bonfire-app/src/gallery.rs` 测试；CLI 用例 `shots-rm`、`shots-rm-short-id`、`shots-rm-purge`、`shots-restore`、`shots-restore-short-id`、`shots-trash`、`shots-trash-empty`、`shots-trash-unknown-game`、`shots-rm-delete-originals-needs-purge` | 0.1.0 | 有效 |
| SHOT-04 | 与磁盘对比与写回 | 对比、写回、重新收集 | CLI 用例 `shots-diff`、`shots-diff-unknown-game`、`shots-write-back`、`shots-write-back-short-id`、`shots-recollect`、`shots-recollect-unknown-game`、`shots-set-off`、`shots-set-bad-value` | 0.1.0 | 有效 |
| SHOT-05 | 未识别的截图 | 对应不上游戏的截图单独成相册 | `crates/bonfire-app/src/unsorted.rs` 测试 | 0.1.0 | 有效 |
