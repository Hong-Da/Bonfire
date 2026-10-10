# CLI 调用文档

命令名 `bonfire`。开发时可在项目根目录用 `cli.bat <参数>` 调用（自动构建并切换 UTF-8 代码页）。

`<game>` 可以是游戏 ID（如 `custom-b041cc3ef897`）或游戏名称（不区分大小写）。
`<ref>` 是快照引用：`HEAD`、`HEAD~2`、分支名、标签名、`heads/<名>`、`tags/<名>`、至少 6 位的哈希前缀（可带 `b3:`）。

## 命令

| 命令 | 作用 |
|---|---|
| `bonfire game add <名称> --path <目录或文件>... [--config-path <路径>]... [--install-dir <目录>]` | 手动添加游戏；`--install-dir` 用于恢复前检测游戏是否在运行 |
| `bonfire game list [--all]` | 列出游戏；`--all` 包括已忽略的 |
| `bonfire game rm <game>` | 移出游戏库（需确认）：手动添加的删除配置，扫描发现的改为忽略；历史快照都保留 |
| `bonfire game include <game>...` / `--all-new` | 把扫描发现的游戏加入游戏库；`--all-new` 加入全部新发现的游戏 |
| `bonfire game ignore <game>...` | 忽略扫描发现的游戏，之后扫描不再作为新发现提示（已在库中的会移出） |
| `bonfire game confirm <game>` | 确认启发式找到的存档位置（写入自定义规则 `custom-rules.yaml`） |
| `bonfire scan [--platform steam\|epic\|gog] [--no-discover] [--no-metadata] [--force-verify]` | 发现游戏：扫描本机平台，列出有存档的游戏（新发现 / 已在库中 / 已忽略），**不会自动加入游戏库**；已在库中的游戏更新存档位置并获取游戏信息；最后检测游戏库中所有游戏的状态 |
| `bonfire rules update` / `rules info` | 下载最新的 Ludusavi 规则库（按 ETag 增量） / 查看当前使用的规则库 |
| `bonfire info <game> [--refresh] [--set 字段=值]... [--unlock 字段]... [--search] [--pick 来源:ID]` | 游戏信息：名称、中文名、开发商、发行商、发行日期、类型、封面、游玩时长、云存档；手动修改的字段加锁 |
| `bonfire status [<game>] [--force-verify]` | 不带游戏时同 `scan`；带游戏时列出变化的文件 |
| `bonfire backup <game>... [-m <备注>] [--tag <标签>] [--force]` | 备份游戏；备注最多 500 字，省略时自动生成 |
| `bonfire backup --all` / `--profile <方案>` | 备份所有有更新或未备份的游戏 / 方案中的游戏 |
| `bonfire backup --auto` | 自动备份：备份所有有变化、已经备份过的游戏（快照标为“定时备份”）并收集截图；与桌面程序的定时备份相同，可供系统计划任务调用。只管理截图的游戏只收集截图 |
| `bonfire backup <game> --new-branch <名称>` / `--move-branch` | 从旧快照恢复后再备份时：指定新分支名 / 移动当前分支（默认自动新建分支） |
| `bonfire log <game> [--branch <分支>] [--grep <关键词>] [--limit <n>]` | 快照历史（`*` 为当前状态对应的快照） |
| `bonfire diff <game> <refA> [<refB>]` | 两个快照的差异；省略 B 时与当前磁盘比较 |
| `bonfire restore <game> <ref> [--path <glob>]... [--full] [--dry-run] [--force]` | 恢复（需确认）；默认只覆盖不删除，`--full` 同时删除快照中没有的文件；`--path` 只恢复匹配的文件 |
| `bonfire undo-restore <game>` | 撤销最近一次恢复（需确认；之后又备份过则不可撤销） |
| `bonfire comment <game> <ref> -m <备注>` | 修改快照备注（保留修改历史） |
| `bonfire branch <game> [<名称> [<ref>]]` | 创建分支；省略名称时列出分支与标签 |
| `bonfire branch <game> -d <名称> [--dry-run]` | 删除分支（需确认）：只属于这个分支的快照一并删除，与其他分支、标签共用的保留；删除的是当前分支时切换到 main（或其他分支）。存档安全保护开启时（`backup.protect_local`，默认），会删掉本机备份快照的分支不能删除（退出码 2） |
| `bonfire tag <game> [<名称> [<ref>]]` | 创建标签（不会被自动清理）；省略名称时列出 |
| `bonfire tag <game> -d <名称>` | 删除标签（快照保留） |
| `bonfire rm <game> <ref> [--dry-run]` | 删除快照（需确认）：后面的快照改接到它的上一个快照上（内容不变、ID 改变，分支、标签与备注随之更新）；分支的最新快照被删时分支退回上一个；带标签的快照受保护；存档安全保护开启时本机备份的快照（快照记录的电脑名与本机相同）不能删除。被删快照占用的空间在之后的仓库清理（GC）时释放 |
| `bonfire profile list` / `save <名称> <game>... [-m <默认备注>]` / `rm <名称>` | 备份方案 |
| `bonfire export <game> [<ref>] (--zip <文件> \| --folder <目录>)` | 导出存档文件：省略 `<ref>` 时导出当前磁盘上的存档；每个存档位置一个子文件夹，附 `bonfire-export.json`；`--folder` 在目录中新建文件夹；拒绝导出到存档位置或仓库中 |
| `bonfire import <game> <zip或文件夹> [--branch <名称>] [--parent <ref>] [--onto <分支>] [-m <备注>] [--dry-run]` | 导入存档文件为新快照（触发方式“导入”），**不写入存档目录**；默认新分支、独立起点，`--parent` 从某个快照分出，`--onto` 接在分支之后；内容与已有快照相同时不重复生成 |
| `bonfire bundle export <game> -o <文件.bonfire> [--branch <分支>]... [--from <ref>] [--to <ref>]` | 导出历史包：默认全部分支与标签；`--branch` 指定分支；`--from`/`--to` 导出一段历史（含两端，导入方需已有更早的快照） |
| `bonfire bundle export (<game> <game>... \| --all) -o <文件.bonfire>` | 导出合集包：多个游戏（或游戏库中全部游戏）的全部分支放在一个文件里；还没有快照的游戏跳过。历史包与合集包加 `--shots` 时同时导出截图相册（默认不包含），导入时加入本地相册 |
| `bonfire bundle import <文件> [--game <game>] [--only <game>]... [--dry-run]` | 导入历史包或合集包：按快照 ID 判断每个分支与本地的关系——新建、相同、后续更新（快进）、本地已包含、分叉（另建 `<分支>-<来源电脑>`，不覆盖本地）；不移动当前快照，不写入存档目录；自定义游戏在不同电脑上 ID 不同时用 `--game` 指定。合集包默认导入全部游戏，`--only` 只导入指定的（ID 或名称）；某个游戏失败不影响其他游戏（退出码 2）。游戏库中没有的游戏自动加入并标为“未安装”：能在本机确定的存档位置照常使用，需要安装目录的位置等装好游戏、扫描后确定（之前恢复会报错） |
| `bonfire shots list <game>` | 列出游戏已备份的截图（ID、拍摄时间、来源、大小、原文件名）与磁盘上还没收集的数量 |
| `bonfire shots sync [<game>...]` | 收集新截图（省略游戏时收集全部游戏）；每次 `backup` 后也会自动收集。来源：Steam 截图、Windows 游戏栏“捕获”文件夹（按文件名中的标题对应）、`game shots-dir` 添加的文件夹；**只读取，不写入截图目录**；内容相同的不重复收集，磁盘上删掉的截图备份中仍保留 |
| `bonfire shots export <game> -o <zip或目录> [--folder] [--id <ID>]... [--since YYYY-MM-DD] [--snapshot <ref>]` | 导出截图（默认全部）为 zip，或 `--folder` 在目录中新建文件夹；`--snapshot` 只导出这个快照之前拍的（拍摄后最早的快照即它） |
| `bonfire shots rm <game> <ID>... [--purge]` | 把截图移入回收站（需确认；ID 前缀至少 6 位），可用 `shots restore` 恢复；`--purge` 彻底删除。只动备份，不动磁盘上的原图；回收站中和彻底删除的截图都不会再被收集 |
| `bonfire shots restore <game> <ID>...` | 从回收站恢复截图 |
| `bonfire shots trash <game> [--empty]` | 列出回收站中的截图；`--empty` 清空回收站（需确认，不能恢复） |
| `bonfire shots found` | 列出有截图、但不在游戏库中的游戏（Steam 截图按 AppID、游戏栏截图按窗口标题） |
| `bonfire shots diff <game>` | 对比截图备份与磁盘上的截图文件夹：两边都有、磁盘上的新截图、被忽略的（在回收站中或已彻底删除，原图还在也不会再收集）、磁盘上已没有的 |
| `bonfire shots write-back <game> [<id>...] [--to <dir>]` | 把备份中有、磁盘上已没有的截图写回原位置（原文件夹不存在时跳过）或 `--to` 指定的文件夹；只写入不存在的文件，不覆盖（需确认） |
| `bonfire shots recollect <game>` | 重新收集被忽略的截图：回收站中的恢复到相册，彻底删除的去掉删除记录后重新收集 |
| `bonfire game add-shots <名称> [--steam <AppID>] [--dir <文件夹>]...` | 加入只管理截图的游戏（不备份存档）；游戏栏截图按名称对应 |
| `bonfire game shots-only <game> on\|off` | 设为只管理截图（不备份、不检查存档，已有快照保留）/ 恢复管理存档 |
| `bonfire game mode <game> [both\|saves\|shots]` | 查看或切换管理方式：存档和截图 / 只管理存档（不收集截图）/ 只管理截图（不备份、不检查存档）；已有的快照与截图都保留 |
| `bonfire shots set <game> on\|off\|auto` | 这个游戏收集 / 不收集截图 / 跟随全局设置（`shots.enabled`） |
| `bonfire game shots-dir add\|rm <game> <文件夹>` | 为游戏添加或移除截图文件夹（游戏自带截图、其他截图工具）；不能是仓库或数据目录；移除后已收集的截图保留 |
| `bonfire fsck [--deep]` | 校验仓库；`--deep` 还原每个文件并校验哈希 |
| `bonfire reindex` | 从仓库重建索引数据库 |

需要确认的命令（`restore`、`undo-restore`、`game rm`）在终端中会提示 `[y/N]`；在脚本或计划任务中必须加 `--yes`，否则以退出码 2 结束。

## 全局参数

| 参数 | 作用 |
|---|---|
| `--repo <DIR>` | 指定存档仓库目录，覆盖配置 |
| `--config <FILE>` | 指定配置文件 |
| `--lang zh-CN\|en` | 输出语言，默认使用配置；配置未设置时跟随系统语言 |
| `--output text\|json` | 输出格式，默认 `text` |
| `--json` | 等同于 `--output json` |
| `-q`, `--quiet` | 减少输出 |
| `-v`, `--verbose` | 在 stderr 输出日志 |
| `-y`, `--yes` | 跳过所有确认提示 |
| `--version` | 显示版本 |

## 输出约定

- `text` 面向人，按所选语言输出。
- `json` 面向程序：结构固定、字段为 camelCase，不随语言变化；结果写 stdout（单行），错误写 stderr，格式为 `{"error": {"code": <退出码>, "kind": "config|usage|not-found|locked|game-running|…", "message": "..."}}`。错误的 `message` 目前为中文。
- `backup` 的 JSON 为 `{"results": [...]}`，每个游戏一项，`result` 为 `created` / `no-changes` / `error`；任一游戏失败时退出码非 0，其他游戏照常备份。
- 日志写入日志目录，只有 `--verbose` 时才同时输出到 stderr，stdout 只放结果。

## 环境变量

| 变量 | 作用 |
|---|---|
| `BONFIRE_HOME` | 数据根目录（默认为程序目录下的 `data`，开发时为项目根目录下的 `data`）：配置、仓库 `repo`、日志、缓存都在其中 |
| `BONFIRE_CONFIG` | 配置文件路径 |
| `BONFIRE_REPO` | 仓库目录 |
| `BONFIRE_LANG` | 语言 |
| `BONFIRE_LOG_LEVEL` | 日志级别 |
| `BONFIRE_OFFLINE` | 设为 `1` 时不发起任何联网请求 |
| `BONFIRE_FAKE_ROOT` | 测试用：把平台安装与系统目录指向伪造的目录 |

## 退出码

| 退出码 | 含义 |
|---|---|
| 0 | 成功 |
| 1 | 一般错误 |
| 2 | 参数错误 |
| 3 | 配置错误或未初始化 |
| 4 | 资源不存在（游戏、快照、引用） |
| 5 | 外部依赖失败（网络、数据库等） |
| 6 | 仓库被锁定（另一个实例正在写入） |
| 7 | 游戏正在运行，恢复被阻止 |

## 示例

<!-- examples:begin 由 tests/cases/cli_cases.toml 生成，请勿手改；运行 test.bat --bless 更新 -->

### 查看版本

```bat
bonfire --version
```

退出码：0

### 未知参数返回退出码 2

```bat
bonfire status --no-such-flag
```

退出码：2

### 配置文件格式错误（JSON 输出）

```bat
bonfire --config tests/inputs/config/bad-syntax.toml --json status
```

退出码：3

### 添加游戏

```bat
bonfire game add 空洞骑士 --path D:/Saves/HollowKnight
```

退出码：0

### 列出游戏（JSON）

```bat
bonfire game list --json
```

退出码：0

### 扫描所有游戏

```bat
bonfire scan
```

退出码：0

### 查看单个游戏的变化（JSON）

```bat
bonfire status 空洞骑士 --json
```

退出码：0

### 备份并写备注

```bat
bonfire backup 空洞骑士 -m 打完螳螂领主
```

退出码：0

### 备份所有有更新的游戏

```bat
bonfire backup --all
```

退出码：0

### 查看历史

```bat
bonfire log 空洞骑士
```

退出码：0

### 快照与当前磁盘比较

```bat
bonfire diff 空洞骑士 HEAD
```

退出码：0

### 把某个快照导出为 zip

```bat
bonfire export 空洞骑士 HEAD --zip D:/Temp/share.zip
```

退出码：0

### 导入别人的存档为新分支

```bat
bonfire import 空洞骑士 D:/Saves/HollowKnight --branch friend
```

退出码：0

### 导出全部分支的历史包

```bat
bonfire bundle export 空洞骑士 -o D:/Temp/hk.bonfire
```

退出码：0

### 导入历史包（预览）

```bat
bonfire bundle import D:/Temp/hk.bonfire --dry-run
```

退出码：0

### 把全部游戏导出为合集包

```bat
bonfire bundle export --all -o D:/Temp/all.bonfire
```

退出码：0

### 导入游戏库中没有的游戏

```bat
bonfire bundle import D:/Temp/all.bonfire
```

退出码：0

### 恢复到上一个快照

```bat
bonfire restore 空洞骑士 HEAD -y
```

退出码：0

### 预览恢复，不写入

```bat
bonfire restore 空洞骑士 HEAD --dry-run
```

退出码：0

### 恢复后撤销，回到恢复前的内容

```bat
bonfire undo-restore 空洞骑士 -y
```

退出码：0

### 修改备注

```bat
bonfire comment 空洞骑士 HEAD -m 白宫之前
```

退出码：0

### 打标签

```bat
bonfire tag 空洞骑士 "Boss 前"
```

退出码：0

### 删除分支

```bat
bonfire branch 空洞骑士 -d 二周目 -y
```

退出码：0

### 存档安全保护：本机备份的快照不能删除

```bat
bonfire rm 空洞骑士 HEAD -y
```

退出码：2

### 删除快照（已关闭存档安全保护）

```bat
bonfire --config tests/inputs/config/no-protect.toml rm 空洞骑士 HEAD -y
```

退出码：0

### 保存备份方案

```bat
bonfire profile save 周末 空洞骑士 -m 周末存档
```

退出码：0

### 校验仓库

```bat
bonfire fsck --deep
```

退出码：0

### 重建索引

```bat
bonfire reindex
```

退出码：0

### 扫描本机平台，识别游戏与存档

```bat
bonfire scan
```

退出码：0

### 确认推测的存档位置

```bat
bonfire game confirm "Indie Quest"
```

退出码：0

### 把发现的游戏加入游戏库

```bat
bonfire game include "Hollow Knight"
```

退出码：0

### 忽略发现的游戏

```bat
bonfire game ignore "Indie Quest"
```

退出码：0

### 查看游戏信息

```bat
bonfire info "Hollow Knight"
```

退出码：0

### 查看规则库

```bat
bonfire rules info
```

退出码：0

### 为游戏添加截图文件夹

```bat
bonfire game shots-dir add 空洞骑士 tests/inputs/shots
```

退出码：0

### 收集新截图

```bat
bonfire shots sync
```

退出码：0

### 列出已备份的截图

```bat
bonfire shots list 空洞骑士
```

退出码：0

### 导出截图为 zip

```bat
bonfire shots export 空洞骑士 -o D:/Temp/shots.zip
```

退出码：0

<!-- examples:end -->
