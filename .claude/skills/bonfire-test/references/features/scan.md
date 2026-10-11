# SCAN 更新检测与状态

变更检测、状态与差异。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| SCAN-01 | 变更检测 | 按大小与修改时间快速判断，必要时校验哈希 | `crates/bonfire-core/src/scan.rs` 测试；CLI 用例 `status-changes-json`、`status-force-verify`、`status-unknown-game` | 0.1.0 | 有效 |
| SCAN-02 | 快照差异 | 与磁盘或另一个快照比较 | CLI 用例 `diff-disk`、`diff-snapshots-json`、`diff-bad-ref` | 0.1.0 | 有效 |
| SCAN-03 | 游戏库视图 | 游戏行状态、筛选、新加入标记 | `crates/bonfire-app/src/library.rs` 测试；`app/src/u10.test.tsx`（U-34 新加入、筛选） | 0.1.0 | 有效 |
