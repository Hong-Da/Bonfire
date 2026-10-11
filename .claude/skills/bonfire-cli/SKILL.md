---
name: bonfire-cli
description: 通过命令行 bonfire 操作篝火（Bonfire）游戏存档备份：查看游戏与状态、备份、查历史、恢复与撤销、分支标签、导入导出、截图、清理历史、仓库加密与校验。用户说“用篝火备份一下空洞骑士”“看看哪些游戏有更新”“恢复到上一个存档”“查一下备注里有 Boss 的快照”“导出这个游戏的存档”“清理历史”“校验仓库”，或 "back up my saves with bonfire", "restore the previous save", "list bonfire games", "run bonfire gc" 时使用。
---

# 篝火命令行（bonfire）

## 前提检查

1. 项目根目录有 `cli.bat`（开发环境）；安装版命令行在 `<安装目录>\cli\bonfire.exe`。开发时也可直接用 `target\debug\bonfire.exe`（先 `cargo build -p bonfire-cli`）。
2. 先运行 `bonfire status --json`：
   - 退出码 3：未初始化或配置错误 → 请用户先运行桌面程序完成首次运行向导，或检查 `data\config.toml`。
   - 仓库已加密且未解锁（错误信息含“仓库已加密”）→ 请用户提供密码，通过环境变量 `BONFIRE_PASSWORD` 传入；不要把密码写进文件或日志。
3. 程序调用一律加 `--json`（等同 `--output json`）；需要确认的命令在非交互环境必须加 `--yes`，但见下方“安全约束”。

## 常用任务速查

| 要做什么 | 命令 |
|---|---|
| 列出游戏 | `bonfire game list --json`（`--all` 含已忽略） |
| 手动添加游戏 | `bonfire game add 空洞骑士 --path "<存档目录>" [--install-dir <目录>]` |
| 移出游戏库 | `bonfire game rm <游戏>`（`--delete-snapshots`、`--delete-shots` 会删除备份） |
| 设置游戏程序（运行检测） | `bonfire game process <游戏> --add Game.exe` |
| 发现本机游戏 | `bonfire scan --json [--platform steam]`，再 `bonfire game include <游戏>...` |
| 哪些游戏有更新 | `bonfire status --json [<游戏>]`（`--force-verify` 重新计算哈希） |
| 备份 | `bonfire backup <游戏>... -m "备注" [--tag 名称]`；全部有更新的：`bonfire backup --all` |
| 按备份方案备份 | `bonfire profile save <方案> <游戏>...`、`bonfire profile list`、`bonfire backup --profile <方案>` |
| 历史 / 搜索备注 | `bonfire log <游戏> [--grep 关键词] [--branch 分支]` |
| 快照差异 | `bonfire diff <游戏> <快照A> [<快照B>]` |
| 恢复 | `bonfire restore <游戏> <快照> --dry-run`，确认后去掉 `--dry-run`；撤销：`bonfire undo-restore <游戏>` |
| 修改备注 | `bonfire comment <游戏> <快照> -m "新备注"` |
| 分支与标签 | `bonfire branch <游戏> [名称 [快照]]`、`bonfire tag <游戏> <名称> [快照]`；删除 `-d` |
| 删除快照（可多个） | `bonfire rm <游戏> <快照>... --dry-run` 先看检查结果 |
| 清理历史 | `bonfire gc --dry-run`，确认后 `bonfire gc --yes` |
| 导出 / 导入存档文件 | `bonfire export <游戏> [快照] --zip <文件>`（或 `--folder <目录>`）、`bonfire import <游戏> <zip或文件夹> [--dry-run]` |
| 篝火包（历史） | `bonfire bundle export <游戏>... -o 文件.bonfire`（`-o 文件.zip --zip [--shots]` 导出 zip）、`bonfire bundle import <文件> [--dry-run]` |
| 导入网上下载的存档 | `bonfire import <游戏> <zip/7z/rar/文件夹> --download [-m 备注]` |
| 截图 | `bonfire shots sync [<游戏>...]`、`bonfire shots list <游戏>`、`bonfire shots export <游戏> -o <zip或目录>` |
| 游戏信息 | `bonfire info <游戏> [--refresh]` |
| 规则库 | `bonfire rules info`、`bonfire rules update` |
| 校验 / 重建索引 | `bonfire fsck [--deep]`、`bonfire reindex` |
| 同步到远程 | `bonfire sync list`、`bonfire sync add <名称> --kind folder|webdav|github|gitee --url <地址>`、`bonfire sync preview <目标> --json`、`bonfire sync push [<目标>...]`；令牌用环境变量 `BONFIRE_SYNC_SECRET` 传入，不要写在命令行 |
| 仓库加密 | `bonfire repo status`、`bonfire repo unlock [--remember]`；加密 / 改密码 / 取消加密见 `docs/cli.md` |

参数、示例与输出字段以 `docs/cli.md` 为准；上表不全或参数不确定时先读它（快照引用：哈希前缀 ≥6 位、`HEAD`、`HEAD~n`、分支名、标签名）。

## 结果解析

成功时 stdout 为一个 JSON 对象；失败时 stderr 为 `{"error":{"code":N,"kind":"...","message":"..."}}`。

| 退出码 | kind | 含义 | 处理建议 |
|---|---|---|---|
| 0 | — | 成功 | — |
| 1 | general | 一般错误（含部分游戏备份失败） | 把 message 原样告诉用户 |
| 2 | usage | 参数错误、需要 `--yes`、密码不对或需要密码、受存档安全保护 | 修正参数；需要确认的操作先问用户 |
| 3 | config | 未初始化、配置错误、仓库版本过高 | 请用户运行桌面程序向导或修正配置 |
| 4 | not-found | 游戏、快照、备份方案不存在 | 用 `game list` / `log` 查正确名称 |
| 5 | external | 联网或数据库失败 | 稍后重试，或 `BONFIRE_OFFLINE=1` |
| 6 | locked | 仓库正被另一个篝火写入 | 等桌面程序的操作结束后重试 |
| 7 | game-running | 游戏正在运行（恢复被阻止）或存档文件被占用 | 请用户退出游戏后重试；不要擅自加 `--force` |

## 安全约束

- 以下操作执行前**必须先向用户确认**，即使命令支持 `--yes`：`restore`、`undo-restore`、`rm`、`gc`、`branch -d`、`tag -d`、`game rm --delete-snapshots/--delete-shots`、`shots rm --purge/--delete-originals`、`repo decrypt`、`repo encrypt`、`sync compact`（改写远程历史）、`sync pull`。同步前先 `sync preview` 给用户看要上传的内容与大小。能 `--dry-run` 的先 dry-run 给用户看结果。
- 恢复会改写真实存档目录：先 `--dry-run`，并提醒用户关闭游戏。
- 测试或演示时绝不对真实游戏存档做恢复、写回；用 `BONFIRE_HOME`、`--repo` 指向临时目录。
- 不读取、不保存、不打印用户的密码；只通过 `BONFIRE_PASSWORD` 环境变量传入。
