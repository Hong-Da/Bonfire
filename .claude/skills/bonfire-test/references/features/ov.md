# OV 总览

总览页：存档的三道防线、待处理与红点通知、存储统计、操作记录、全部游戏的截图与磁盘对比。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| OV-01 | 操作记录 | 各项操作与失败原因记入本机记录；按类型、游戏、文字筛选，翻页；补出以前的历史；保留 180 天 | `crates/bonfire-app/src/activity.rs` 测试；CLI 用例 `activity-after-backup`、`activity-failed-none`；`app/src/overview.test.tsx`（操作记录） | 0.3.0 | 有效 |
| OV-02 | 存储统计 | 仓库按存档快照 / 截图 / 其他统计，每个游戏的占用，原始大小，增长曲线，磁盘剩余 | `crates/bonfire-core/src/usage.rs`、`crates/bonfire-app/src/overview.rs` 测试；CLI 用例 `stats` | 0.3.0 | 有效 |
| OV-03 | 需要处理的问题 | 同步失败、备份失败、校验问题；忽略与稍后提醒；红点（导航、任务栏、托盘）与系统通知 | `overview.rs`（`attention_lists_failures_and_snoozes`）；`app/src-tauri/src/attention.rs` 测试（红点图标）；`app/src/overview.test.tsx`（待处理）；系统通知手动验收 | 0.3.0 | 有效 |
| OV-04 | 截图与磁盘 | 全部游戏对比，一键收集、一键写回（不覆盖） | `overview.rs`（`compare_collect_and_write_back_all`）；CLI 用例 `shots-compare-empty`；`app/src/overview.test.tsx`（截图） | 0.3.0 | 有效 |
