# BAK 备份

快照式备份、去重、备份方案。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| BAK-01 | 备份 | 生成快照，未变化文件不重复存储，多文件并行入库 | `crates/bonfire-core/src/store.rs`、`crates/bonfire-app/src/backup.rs` 测试；CLI 用例 `backup-ok`、`backup-json`、`backup-no-changes`、`backup-all`、`backup-en`、`backup-no-target`、`backup-bad-tag` | 0.1.0 | 有效 |
| BAK-02 | 备份方案 | 保存常用的游戏组合并按方案备份 | CLI 用例 `profile-save-ok`、`profile-list-json`、`profile-backup`、`profile-list-extra-arg`、`profile-save-unknown-game`、`profile-rm-missing`、`profile-rm-ok` | 0.1.0 | 有效 |
| BAK-03 | 崩溃一致性 | 写入任意步骤中断不损坏已有快照 | `tests/cli/crash.rs` | 0.1.0 | 有效 |
| BAK-04 | 备份对话框 | 逐个游戏写备注、运行中提醒 | `app/src/App.test.tsx` | 0.1.0 | 有效 |
