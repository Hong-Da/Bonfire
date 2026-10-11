# CLEAN 历史清理

保留策略、垃圾回收、空间来源。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| CLEAN-01 | 保留策略 | 最近 N 个、每天 / 每周 1 个，永远保留标签等 | `crates/bonfire-app/src/retention.rs` 测试；`app/src/u10.test.tsx`（M4-4 历史清理） | 0.1.0 | 有效 |
| CLEAN-02 | 垃圾回收与空间来源 | 标记-清除，按来源归类可释放空间 | `crates/bonfire-core/src/gc.rs` 测试；CLI 用例 `gc-preview`、`gc-json` | 0.1.0 | 有效 |
