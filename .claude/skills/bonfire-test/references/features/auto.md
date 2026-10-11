# AUTO 自动备份与运行检测

定时、变化后、退出游戏后自动备份；游戏运行检测。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| AUTO-01 | 定时与变化后自动备份 | 只备份有变化、备份过的游戏 | `crates/bonfire-app/src/auto.rs` 测试；CLI 用例 `backup-auto`、`backup-auto-with-game`；`app/src/auto.test.tsx` | 0.1.0 | 有效 |
| AUTO-02 | 退出游戏后自动备份 | 游戏退出 10 秒后备份，运行中推迟 | `crates/bonfire-app/src/auto.rs` 测试（exit_watcher、exit_backup） | 0.1.0 | 有效 |
| AUTO-03 | 游戏运行检测 | 按安装目录或游戏程序（进程名） | `crates/bonfire-app/src/process.rs` 测试；CLI 用例 `game-process`、`game-process-clear`；`app/src/u10.test.tsx`（M4-2 游戏程序） | 0.1.0 | 有效 |
