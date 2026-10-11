# UI 桌面界面

首次运行向导、设置、多语言、确认框、托盘。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| UI-01 | 首次运行向导 | 语言、仓库、平台、游戏信息 | `app/src/App.test.tsx`（向导） | 0.1.0 | 有效 |
| UI-02 | 设置页 | 仓库、界面、自动备份、截图、清理、加密等设置 | `app/src/u10.test.tsx`、`app/src/auto.test.tsx` | 0.1.0 | 有效 |
| UI-03 | 多语言 | 中文 / English 文案齐全 | `app/src/i18n.test.ts`；`crates/bonfire-app/src/i18n.rs` 测试 | 0.1.0 | 有效 |
| UI-04 | 确认框与界面状态 | 应用内确认框；筛选等界面状态保存 | `app/src/confirm.test.tsx`；`crates/bonfire-app/src/ui_state.rs` 测试 | 0.1.0 | 有效 |
