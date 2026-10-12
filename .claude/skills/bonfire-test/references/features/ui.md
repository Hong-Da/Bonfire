# UI 桌面界面

首次运行向导、设置、多语言、确认框、托盘。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| UI-01 | 首次运行向导 | 语言、从远程获取（可跳过）、仓库、平台、自动备份、游戏信息；在设置中重新运行 | `app/src/App.test.tsx`（向导）；`crates/bonfire-app/src/sync/tests.rs`（`first_run_restore_without_app`） | 0.1.0 | 有效 |
| UI-02 | 设置页 | 仓库、界面、自动备份、截图、清理、加密等设置 | `app/src/u10.test.tsx`、`app/src/auto.test.tsx` | 0.1.0 | 有效 |
| UI-03 | 多语言 | 中文 / English 文案齐全 | `app/src/i18n.test.ts`；`crates/bonfire-app/src/i18n.rs` 测试 | 0.1.0 | 有效 |
| UI-04 | 确认框与界面状态 | 应用内确认框；筛选等界面状态保存 | `app/src/confirm.test.tsx`；`crates/bonfire-app/src/ui_state.rs` 测试 | 0.1.0 | 有效 |
| UI-05 | 安装与卸载 | NSIS 安装包、升级保留数据、卸载时询问是否删除数据并提示远程备份不会删除 | `tests/installer/check-installer.ps1`（本机未安装篝火时运行；界面卸载的询问手动验收） | 0.2.4 | 有效 |
| UI-06 | 功能导览 | 8 个常用功能的卡片，去试试跳到对应页面后收起到右下角、可继续；设置中查看；升级用户在总览提示 | `app/src/App.test.tsx`（功能导览） | 0.4.0 | 有效 |
| UI-07 | 恢复出厂设置 | 清除本机设置与数据，可选删除备份仓库；输入确认文字才执行；重启后执行 | `crates/bonfire-app/src/reset.rs` 测试；`app/src/App.test.tsx`（恢复出厂设置）；实际执行手动验收 | 0.4.2 | 有效 |
