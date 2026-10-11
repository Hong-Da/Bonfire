# SYNC 同步备份

把仓库、最新存档、截图与篝火设置同步到 GitHub、Gitee、WebDAV、网盘、远程目录、指定目录，并能从远程恢复。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| SYNC-01 | 同步目标设置 | 六类目标、多目标、令牌与密码只存在本机、删除目标 | `crates/bonfire-app/src/sync/tests.rs`（`shots_only_and_secrets_and_removal`）；CLI 用例 `sync-unknown-target`；`app/src/sync.test.tsx`（添加 Gitee 目标、每个游戏备份到了哪里的筛选与搜索） | 0.2.0 | 有效 |
| SYNC-02 | 增量同步与内容 | 完整仓库 / 最新存档 / 只同步截图；截图与篝火设置可选；按指纹增量上传与删除 | `sync/tests.rs`（`repo_sync_is_incremental_and_restorable`、`latest_saves_with_size_limit_and_deletions`）；CLI 用例 `sync-folder` | 0.2.0 | 有效 |
| SYNC-03 | 大小检查 | 单文件上限排除并列出、总大小 80% 提醒与超出时阻止、解决办法 | `sync/tests.rs`（`total_limit_blocks_and_git_defaults`）；CLI 用例 `sync-preview`；`app/src/sync.test.tsx`（超过总大小上限） | 0.2.0 | 有效 |
| SYNC-04 | WebDAV 与 GitHub / Gitee | WebDAV 建文件夹与列出；Git 分批推送、压缩远程仓库、公开仓库检查 | `sync/tests.rs`（`webdav_sync_and_restore` 模拟 WebDAV 服务、`git_sync_push_restore_and_compact` 本地裸仓库）；真实 Gitee 需要令牌，手动验收 | 0.2.0 | 有效 |
| SYNC-06 | 失败原因、自动重试与忽略 | 失败按原因分类；连不上目标时 5 / 15 / 60 分钟自动重试；可忽略连不上、超过总大小、单文件超限 | `sync/tests.rs`（`network_failures_retry_with_backoff`、`total_limit_blocks_and_git_defaults` 忽略部分）；`app/src/sync.test.tsx`（失败原因与忽略） | 0.2.3 | 有效 |
| SYNC-05 | 自动同步与从远程恢复 | 备份后 / 每天 / 每隔几小时同步；从远程下载完整仓库、套用远程设置 | `sync/tests.rs`（`schedules_and_settings_apply`、各目标的恢复断言） | 0.2.0 | 有效 |
