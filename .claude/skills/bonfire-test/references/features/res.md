# RES 恢复与撤销

恢复预览、保护快照、回滚与撤销。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| RES-01 | 恢复 | 预览差异、全部或部分恢复、失败回滚 | `crates/bonfire-app/src/restore.rs` 测试；CLI 用例 `restore-ok`、`restore-dry-run`、`restore-partial`、`restore-needs-yes`、`restore-no-parent` | 0.1.0 | 有效 |
| RES-02 | 撤销恢复 | 回到恢复前的状态 | `crates/bonfire-app/src/restore.rs` 测试；CLI 用例 `undo-restore-nothing`、`undo-after-restore` | 0.1.0 | 有效 |
