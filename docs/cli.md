# CLI 调用文档

命令名 `bonfire`。开发时可在项目根目录用 `cli.bat <参数>` 调用（自动构建并切换 UTF-8 代码页）。

`<game>` 可以是游戏 ID（如 `custom-b041cc3ef897`）或游戏名称（不区分大小写）。
`<ref>` 是快照引用：`HEAD`、`HEAD~2`、分支名、标签名、`heads/<名>`、`tags/<名>`、至少 6 位的哈希前缀（可带 `b3:`）。

## 命令

| 命令 | 作用 |
|---|---|
| `bonfire game add <名称> --path <目录或文件>... [--config-path <路径>]... [--install-dir <目录>]` | 手动添加游戏；`--install-dir` 用于恢复前检测游戏是否在运行 |
| `bonfire game list` | 列出游戏 |
| `bonfire game rm <game>` | 移除游戏（需确认；历史快照保留在仓库中） |
| `bonfire scan [--force-verify]` | 检测所有游戏的状态：未备份 / 有更新 / 已是最新 |
| `bonfire status [<game>] [--force-verify]` | 不带游戏时同 `scan`；带游戏时列出变化的文件 |
| `bonfire backup <game>... [-m <备注>] [--tag <标签>] [--force]` | 备份游戏；备注最多 500 字，省略时自动生成 |
| `bonfire backup --all` / `--profile <方案>` | 备份所有有更新或未备份的游戏 / 方案中的游戏 |
| `bonfire backup <game> --new-branch <名称>` / `--move-branch` | 从旧快照恢复后再备份时：指定新分支名 / 移动当前分支（默认自动新建分支） |
| `bonfire log <game> [--branch <分支>] [--grep <关键词>] [--limit <n>]` | 快照历史（`*` 为当前状态对应的快照） |
| `bonfire diff <game> <refA> [<refB>]` | 两个快照的差异；省略 B 时与当前磁盘比较 |
| `bonfire restore <game> <ref> [--path <glob>]... [--full] [--dry-run] [--force]` | 恢复（需确认）；默认只覆盖不删除，`--full` 同时删除快照中没有的文件；`--path` 只恢复匹配的文件 |
| `bonfire undo-restore <game>` | 撤销最近一次恢复（需确认；之后又备份过则不可撤销） |
| `bonfire comment <game> <ref> -m <备注>` | 修改快照备注（保留修改历史） |
| `bonfire branch <game> [<名称> [<ref>]]` | 创建分支；省略名称时列出分支与标签 |
| `bonfire tag <game> [<名称> [<ref>]]` | 创建标签（不会被自动清理）；省略名称时列出 |
| `bonfire profile list` / `save <名称> <game>... [-m <默认备注>]` / `rm <名称>` | 备份方案 |
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
| `BONFIRE_HOME` | 把配置、仓库、日志、缓存都放在该目录下（同便携模式） |
| `BONFIRE_CONFIG` | 配置文件路径 |
| `BONFIRE_REPO` | 仓库目录 |
| `BONFIRE_LANG` | 语言 |
| `BONFIRE_LOG_LEVEL` | 日志级别 |

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

<!-- examples:end -->
