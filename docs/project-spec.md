# 篝火 Bonfire 开发规范

> 生成日期：2026-10-09
> 读取日期：2026-10-08（任务文档）
> last_synced_commit：（阶段三生成初始版本后填写 v0.1.0）
> 每条决定后标注【默认】【建议：原因】或【用户修改】。
> 选项记录：DevelopDocs/project-config/config.json（确认方式：网页）

## 项目概述

篝火 Bonfire 是一款 Windows 桌面端游戏存档备份工具：自动扫描本机各平台的游戏存档、判断哪些有更新，由用户选择备份，并以类 Git 的快照模型保存完整历史，支持随时恢复到任意版本。面向 PC 玩家，首版运行在 Windows 10/11 64 位，提供桌面 GUI（Tauri）与 CLI，两者共用 Rust 核心库。

## 开发文档

| 文件 | 用途 | 说明 |
|---|---|---|
| `DevelopDocs/游戏存档备份软件 · 开发设计文档.md` | 任务文档 | 主文档（唯一），需求以此为准 |
| `DevelopDocs/project-config/` | 选项记录 | 三轮确认的建议值与用户选择 |
| `DevelopDocs/review/<…>/` | 盘点 | 需求盘点结果（阶段三及之后生成） |

验收标准：任务文档没有独立的“验收标准”一章，经用户确认（2026-10-08）以 **10.1 非功能指标 + 10.4 各里程碑验收 + P0 全部交付** 为准。

## 需求清单

| 编号 | 需求 | 来源 | 里程碑 | 对应功能 | 状态 | 备注 |
|---|---|---|---|---|---|---|
| REQ-001 | 存档扫描：识别平台与游戏、定位存档路径、手动添加自定义游戏/路径、自动获取游戏信息 | 任务文档 §2 F1、§4 | M1（手动部分）、M2、M4（其余平台） | （阶段三补齐） | 待开发 | P0 |
| REQ-002 | 更新检测：已修改/新增/已删除/未变化/从未备份 | §2 F2、§5 | M1 | | 待开发 | P0 |
| REQ-003 | 选择性备份：勾选、全选/反选、按平台筛选、备份方案 | §2 F3、§8.1 | M1、M3 | | 待开发 | P0 |
| REQ-004 | 版本历史：时间线、快照差异、分支与标签 | §2 F4、§6 | M1、M3 | | 待开发 | P0 |
| REQ-005 | 快照备注：逐游戏备注、事后修改、全文搜索 | §2 F5、§6.3、§8.3 | M1、M3 | | 待开发 | P0 |
| REQ-006 | 存档恢复：全部/部分恢复、保护快照、一键撤销 | §2 F6、§8.2 | M1、M3 | | 待开发 | P0 |
| REQ-007 | 游戏运行保护：运行中警告或阻止恢复 | §2 F7 | M1（简化版）、M4 | | 待开发 | P1；简化版提前到 M1（R2-Q3） |
| REQ-008 | 自动备份：定时或游戏退出后自动备份 | §2 F8 | M4 | | 待开发 | P1 |
| REQ-009 | 历史清理：保留策略、标签永不清理、垃圾回收 | §2 F9、§6.6 | M4 | | 待开发 | P1 |
| REQ-010 | 导入/导出单游戏历史 | §2 F10 | M5 | | 待开发 | P2 |
| REQ-011 | 仓库可选 AES-256-GCM 加密 | §10.1 | M5 | | 待开发 | P2 |
| REQ-012 | VSS 卷影复制读取独占锁定文件 | §5 | M5 | | 待开发 | P2 |
| REQ-013 | Linux / Steam Deck（Proton）支持 | §1、§10.4 M5 | M5 | | 待开发 | P2 |

## 待确认

- （暂无。阶段一的待确认事项已全部确认，见“已确认的待确认点”。）

## 开发记录

- M1-1（2026-10-09）：与方案的出入——Rust edition 改用 2024（cargo 默认，rust-version 1.89，仓库锁使用标准库 File::try_lock）；CLI `--help` 说明文字暂为中英并列，随语言切换在 M1-7 实现；GUI 主题与语言暂存于 WebView 本地存储，M3-1 与应用配置同步。
- M1-2（2026-10-09）：与文档 6.1 的出入——对象按类型分目录存放（`objects/chunks|blobs|trees|snapshots/ab/cdef…`），因为小文件的 Blob ID（整个文件的 BLAKE3）与其唯一 Chunk 的 ID 相同，同目录会冲突；临时文件在 `objects/tmp/`。每个对象文件是带 XXH64 校验和的 zstd 帧，Blob 的元数据也压缩存放。新增故障注入点 `BONFIRE_FAULT_INJECT=<点>[:<第 N 次>]`（before-rename / after-chunk / after-blob / after-tree / after-snapshot），供崩溃一致性测试使用。
  - 待观察：每个对象写入后都 `fsync`，对吞吐（≥ 200 MB/s）的影响在 M4-5 基准测试中确认，必要时改为批量落盘。
- M1-3（2026-10-09）：HEAD 存为 JSON `{"branch": "main", "detached": <快照>}`，`detached` 表示“已从该快照恢复”，下次备份以它为父快照并触发分支选择（文档 6.4、8.2）。分支名与标签名允许中文，但须是合法的 Windows 文件名、不含 `~ ^`，且按不区分大小写判重。引用解析支持 `HEAD`、`HEAD~n`、分支名、标签名、`heads/<名>`、`tags/<名>`、≥ 6 位哈希前缀；同名分支与标签或多个前缀匹配时报歧义。所有写操作在类型上要求持有仓库锁；分支更新为比较并交换。reflog 与注记时间为 UTC（`jiff`）。
- M1-4（2026-10-09）：备注全文搜索用 FTS5 `trigram` 分词（支持中文子串、不区分大小写），少于 3 个字符的查询退回 `LIKE`。大小已变的文件直接判为已修改，哈希留到备份时计算。`reindex` 重建快照、父子关系、refs 镜像与变更检测缓存（取当前状态对应快照的清单）；`games` 与 `save_locations` 由应用层从 `games.toml` 恢复。

## N 名称与标识

- N1 项目名称：中文名 **篝火**，英文名 **Bonfire**，项目标识 `bonfire`，CLI 命令 `bonfire`，Rust crate 前缀 `bonfire-`（方案选定：任务文档中的名称）【默认】
  - 注意：已有同名社交应用与 Python 包；CLI 命令名在本机未被占用。
- Logo：方案二“回溯之火”，`assets/brand/logo.svg`，主色 `#e8590c`（同时作为界面强调色），辅色 `#ffd166`、白色【用户选择】

## A 基线

- A1 版本管理：git【默认】
- A2 Python 环境：不使用。Rust 用 Cargo（`rust-toolchain.toml` 固定稳定版），前端用 pnpm（`pnpm-lock.yaml` 入库）【建议：技术栈为 Rust + Tauri（文档 10.2）】
- A3 运行平台：仅 Windows 10/11 64 位；代码层面保持跨平台写法，为 M5 Linux/macOS 预留【建议：文档 §1】
- A4 文本编码：UTF-8（bat 文件 UTF-8 无 BOM + `chcp 65001` + CRLF；`.ps1` UTF-8 带 BOM）【默认】
- A5 编码行为准则：Karpathy guidelines（已安装：插件 andrej-karpathy-skills）【默认】
- A6 一键运行脚本：`start.bat`（检查 Rust/MSVC/Node/pnpm → 安装前端依赖 → 构建 → 启动；首次运行由 GUI 向导引导）【默认】
- A7 版本号：语义化版本，初始 0.1.0（用户已确认），写在 Cargo workspace 的 `workspace.package.version` 与 `app/package.json`、`tauri.conf.json`，三处一致【默认】
- A8 文档语言：跟随对话语言（当前为中文）【默认】

## B 项目形态

- B1 项目类型：Web 应用 → 实际为 **Tauri 2 桌面应用**（界面为 Web 技术，因此按 Web 应用启用主题、页面设计等选项）【默认，对话中确认】
- B2 界面框架：Tauri 2 + React + TypeScript + Vite【用户修改：选择“帮我选择”，2026-10-09 对话确认】

## C 界面与输出

- C1 界面语言：中文、English，可切换，首次跟随系统语言，无法匹配时中文；GUI 与 CLI 共用文案键【用户修改：去掉葡语】
- C2 报告语言：不适用（无报告）
- C3 界面主题：自动 / 明 / 暗三态切换，初始自动【默认】
- C4 页面设计 skill：frontend-design（已安装）【默认】

## D 数据与运行

- D1 数据存储：SQLite（`rusqlite`，含 FTS5）。数据库 `bonfire.db` 位于仓库目录内，只作索引，可由 `bonfire reindex` 从对象库重建（文档 §7）【默认，与文档一致】
- D2 配置文件：TOML。应用配置 `%APPDATA%\Bonfire\config.toml`；仓库参数 `<repo>/config.toml`；游戏配置备份 `<repo>/games.toml`；仓库默认 `%LOCALAPPDATA%\Bonfire\repo`（可配置）。仓库提交 `config/config.example.toml`。所有参数都有默认值，缺失配置时程序仍能启动【默认；位置为 R2-Q4 确认】
  - 便携模式：程序目录下存在 `portable` 标记文件时，配置、日志、缓存、仓库都放在程序目录旁的 `data/` 下【R2-Q2 确认】
  - 配置优先级：环境变量（`BONFIRE_*`）> 配置文件 > 代码默认值；所有配置项在 `bonfire-app` 的一处定义。
- D3 默认监听：127.0.0.1（用户已确认）。桌面应用本身不监听网络端口；仅开发时的 Vite 开发服务器监听本机，端口按 Tauri 惯例为 1420【默认】
- D4 首次运行向导：GUI 向导（Tauri 窗口内）：语言 → 仓库位置 → 平台开关 → 是否备份配置文件 → 游戏信息联网获取与 API Key → 汇总确认。CLI 在无配置时直接使用默认值，不进入交互【默认】
- D5 日志：INFO / 按日期轮转 / 保留 14 天，目录 `%LOCALAPPDATA%\Bonfire\logs`（便携模式在 `data/logs`），使用 `tracing` + `tracing-appender`，不记录 API Key【默认】

## E 调用接口

- E1 CLI：开发，命令名 `bonfire`（Rust + `clap`）。默认输出文本；`--json` 与 `--output json` 都支持（`--json` 为简写）【默认；JSON 参数写法 Q1 确认】
  - 退出码：0 成功 / 1 一般错误 / 2 参数错误 / 3 配置错误或未初始化 / 4 资源不存在 / 5 外部依赖失败 / 6 仓库被锁定（另一实例正在写入）/ 7 游戏正在运行，恢复被阻止
  - 全局参数：`--repo <路径>`、`--config <路径>`、`--lang zh-CN|en`、`--output text|json`、`--json`、`--quiet`、`--verbose`、`--yes`、`--version`
  - 快照引用：哈希前缀（≥ 6 位）、`HEAD`、`HEAD~n`、分支名、标签名
- E2 远程 API：不开发【用户修改：2026-10-09 对话确认，文档无 HTTP 接口】

### 已确认的能力清单

| 能力 | CLI 命令 | 调用场景 | 优先级 |
|---|---|---|---|
| 游戏管理 | `bonfire game add\|list\|rm` | M1 只支持手动路径；模拟器/独立游戏 | 必要 |
| 扫描与更新检测 | `bonfire scan [--platform] [--force-verify]` | 计划任务定时检查有无更新 | 必要 |
| 查看状态 | `bonfire status [<game>]` | 脚本判断是否需要备份 | 必要 |
| 备份 | `bonfire backup <game>... [-m] [--profile] [--tag] [--force]` | 计划任务、脚本批量备份 | 必要 |
| 历史 / 搜索备注 | `bonfire log <game> [--branch] [--grep]` | 查找某个存档点 | 必要 |
| 快照差异 | `bonfire diff <game> <snapA> [<snapB>]` | 恢复前确认变化 | 必要 |
| 恢复 | `bonfire restore <game> <ref> [--path] [--full] [--dry-run]` | 回到某个存档点 | 必要 |
| 撤销上次恢复 | `bonfire undo-restore <game>` | 恢复错了时找回 | 必要 |
| 修改备注 | `bonfire comment <game> <snap> -m` | 补写备注 | 必要 |
| 分支与标签 | `bonfire branch\|tag <game> <name> [<snap>]` | 多周目、重要节点 | 必要 |
| 游戏信息 | `bonfire info <game> [--refresh]` | 查看封面、时长等元数据 | 建议 |
| 仓库校验 | `bonfire fsck` | 定期校验仓库完整性 | 建议 |
| 重建索引 | `bonfire reindex` | SQLite 损坏后从对象库恢复 | 建议 |
| 规则库更新 | `bonfire rules update` | 手动刷新 Ludusavi 规则 | 建议 |
| 历史清理与 GC | `bonfire gc [--dry-run]` | 释放空间（P1） | 建议 |
| 备份方案管理 | `bonfire profile list\|save\|rm` | 复用常用游戏组合 | 可选 |
| 导入/导出 | `bonfire export <game> -o <file>` / `bonfire import <file>` | 迁移或分享（P2） | 可选 |

17 项全部开发（第二轮确认）。

## 实现方案

### 技术选型

| 层 | 选型 | 理由 |
|---|---|---|
| 核心库 | Rust（stable，edition 2024）：`blake3`、`zstd`、`fastcdc`、`rusqlite`（bundled，启用 FTS5）、`globset`、`serde`/`serde_json`、`toml`、`fs4`（文件锁）、`rayon`（并行哈希）、`thiserror` | 文档 10.2；IO 与并发性能好，crate 成熟 |
| 存档发现 | `keyvalues-parser` 或自写 VDF 解析、`winreg`、`serde_yaml`（Ludusavi manifest）、`sysinfo`（进程检测） | 文档 §4 |
| 游戏信息 | `reqwest`（rustls）、`image` + `webp` 编码 | 文档 4.3；联网失败不阻塞 |
| CLI | `clap`（derive）、`anyhow` | 文档 10.2；与 GUI 共用核心 |
| 桌面 UI | Tauri 2 + React 18 + TypeScript + Vite；状态管理 Zustand；路由 React Router；i18n 用 i18next | 文档 10.2；安装包小 |
| 日志 | `tracing`、`tracing-subscriber`、`tracing-appender` | 按日期轮转 |
| 测试 | `cargo test`（单元、集成、CLI、文档），CLI 用 `assert_cmd` 执行真实二进制；前端 Vitest + Testing Library | T1 |
| 安装包 | Tauri bundler → NSIS（按用户安装） | Q2 |

### 模块划分

Cargo workspace，引擎层不感知界面：

| crate / 目录 | 职责 |
|---|---|
| `crates/bonfire-core` | 引擎·版本存储：对象库（Chunk/Blob/Tree/Snapshot）、FastCDC 分块、zstd、原子写、仓库锁、refs/HEAD/reflog/notes、引用解析、SQLite 索引、ScanIndex 变更检测、fsck、GC |
| `crates/bonfire-discovery` | 引擎·存档发现：PlatformAdapter（Steam/Epic/GOG/…）、路径变量与 glob、Ludusavi 规则库、启发式探测、注册表导出、MetadataProvider |
| `crates/bonfire-app` | 应用服务层：扫描、备份、恢复、历史、清理、元数据服务；配置（唯一定义）、日志、i18n 文案、进程检测、便携模式与路径解析 |
| `crates/bonfire-cli` | CLI：参数解析与输出格式化，调用 `bonfire-app`，二进制名 `bonfire` |
| `app/src-tauri` | Tauri 壳：命令层（调用 `bonfire-app`）、事件推送（扫描进度）、托盘与自动备份守护（同一进程，关闭窗口时销毁 WebView） |
| `app/src` | React 前端：游戏库、备份确认、游戏详情/历史、设置、首次运行向导 |

### 关键流程

- **扫描**：适配器列出游戏 → 规则库解析存档位置 → 逐游戏按 ScanIndex 做元数据比对，元数据变化的文件计算 BLAKE3 → 输出游戏级状态，逐游戏推送到界面（文档 §5）。
- **备份**：读文件（共享读）→ FastCDC 分块 → 已存在的 Chunk 跳过 → Chunk → Blob → Tree → Snapshot → 更新 ref + reflog + ScanIndex；与上一快照 Tree 相同时不建快照（文档 8.1）。
- **恢复**：差异预览 → 进程检测（M1 简化版警告）→ 当前状态与最新快照不同时先建 `pre-restore` 保护快照 → 写临时文件 → 全部成功后逐个原子替换，失败用保护快照回滚 → 记录 `operations`，HEAD 标记“已从 X 恢复”（文档 8.2）。
- **数据表**：`games`、`save_locations`、`scan_index`、`snapshots`（+ FTS5）、`snapshot_parents`、`refs`、`backup_profiles`、`operations`、`game_metadata`（文档 §7、4.3）。

### 风险与难点

| 风险 | 应对 |
|---|---|
| 崩溃一致性（断电不损坏快照） | 固定写入顺序 + 临时文件原子重命名 + ref 最后更新；测试中用故障注入（在写入各步骤后强杀子进程）反复验证，并跑 fsck |
| Windows 文件被游戏独占锁定 | 共享读模式；读取失败的文件在结果中列出；VSS 留到 M5 |
| mtime 精度（FAT32/网络盘） | racy-git 处理：mtime 与索引写入时间差 < 2 秒的文件重算哈希 |
| 原子替换在 Windows 上的限制 | 使用 `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`（`std::fs::rename` 在 Windows 上即此语义），同卷临时文件 |
| 本机 Steam 游戏数量不足 50 款 | M2 识别率以本机实际已装游戏为准（Q5），并用伪造平台目录覆盖各适配器 |
| 无 IGDB / SteamGridDB API Key | 用模拟数据测试（Q6），真实联调留给用户 |
| Ludusavi manifest 体积较大 | 构建时转为压缩的紧凑格式内置；启动时按 ETag 增量更新（Q3） |
| MSVC 生成工具缺失 | 开发机已由 Claude 用 winget 安装；`start.bat` 检测并给出安装提示 |

## 里程碑与开发阶段

> 规划来源：开发文档中已有（10.4），按需求编号与开发阶段细化；项目规模：大（13 条需求、GUI + CLI + 守护进程、多个外部平台）

### 里程碑

| 编号 | 目标 | 包含需求 | 交付物 | 验收方式 | 状态 |
|---|---|---|---|---|---|
| M1 | 核心库：对象库、快照、refs、变更检测、备份与恢复；手动添加路径；CLI 可用 | REQ-001（手动部分）、002–006、007（简化版） | Rust 核心库 + `bonfire` CLI | 随机断电（强杀 + 故障注入）测试无快照损坏；fsck 通过；CLI 端到端测试通过；用户验收 | 进行中 |
| M2 | 存档发现：Steam、Epic、GOG + Ludusavi 规则库 + 游戏信息 | REQ-001 | `bonfire scan` 自动识别并补全信息 | 本机 Steam 已装游戏只读扫描识别率 ≥ 90%；伪造平台目录测试通过 | 未开始 |
| M3 | GUI MVP：游戏库、备份确认、历史时间线、恢复；P0 全部交付 | REQ-001–006 | Tauri 桌面应用 + NSIS 安装包 | P0 在 GUI 中全部可用；前端测试通过；用户本机安装验收 | 未开始 |
| M4 | 完善：其余平台、运行保护、自动备份、保留策略与 GC | REQ-001、007–009 | 全平台识别、托盘自动备份、历史清理 | 文档 10.1 指标：200 游戏增量扫描 < 3 秒、吞吐 ≥ 200 MB/s、托盘内存 < 80 MB；用户验收 | 未开始 |
| M5 | 扩展（P2）：导入导出、加密、VSS、Linux/Steam Deck | REQ-010–013 | P2 功能 | 各功能测试通过；用户验收 | 未开始 |

### 开发阶段

| 编号 | 内容 | 完成标准 | 对应测试 | 依赖 | 规模 | 状态 |
|---|---|---|---|---|---|---|
| M1-1 | 骨架：Cargo workspace、Tauri 空壳、pnpm 前端、配置、日志、图标导出、start/cli/test 脚本、测试框架 | `start.bat` 启动空窗口；`test.bat` 跑通 cargo test 与 Vitest | 单元：配置加载与默认值；文档：目录结构一致 | — | M | 已完成（2026-10-09） |
| M1-2 | 对象库：Chunk/Blob/Tree/Snapshot、BLAKE3、zstd、FastCDC、原子写、仓库锁、仓库版本号 | 往返一致；相同内容去重；锁防双写 | 单元（含子进程故障注入的崩溃测试） | M1-1 | L | 已完成（2026-10-09） |
| M1-3 | Refs：分支/标签/HEAD、reflog、notes、引用解析 | 解析与 reflog 正确 | 单元 | M1-2 | M | 已完成（2026-10-09） |
| M1-4 | SQLite 索引（FTS5）、ScanIndex 变更检测（racy-git、强制校验、并行）、reindex | 五种状态判定正确；删库后可重建 | 单元 | M1-2、M1-3 | M | 已完成（2026-10-09） |
| M1-5 | 备份服务：手动添加游戏、逐游戏备注与默认备注、无变化不建快照、`--force` | 备份后为“已是最新”；备注可搜索 | 单元、CLI | M1-4 | M | 未开始 |
| M1-6 | 恢复服务：预览、部分/完全恢复、保护快照、原子替换与回滚、撤销、分支选择、简化版进程检测 | 恢复与撤销往返一致；失败自动回滚 | 单元、CLI | M1-5 | L | 未开始 |
| M1-7 | CLI 全部 M1 命令、JSON 输出、退出码、`docs/cli.md`；崩溃一致性测试与 fsck | CLI 用例全部通过；故障注入后 fsck 无错误 | CLI、文档 | M1-5、M1-6 | M | 未开始 |
| M2-1 | PlatformAdapter 接口、路径变量、glob、注册表导出 .reg、2 GB 告警 | 单测通过 | 单元 | M1 | M | 未开始 |
| M2-2 | Steam 适配器（vdf/acf、steamId3、云存档标记、游玩时长） | 伪造目录测试通过；本机只读扫描列出已装游戏 | 单元、CLI | M2-1 | M | 未开始 |
| M2-3 | Epic、GOG 适配器 | 伪造目录测试通过 | 单元 | M2-1 | M | 未开始 |
| M2-4 | Ludusavi 规则库：内置快照 + 增量更新、用户规则覆盖、启发式探测 | 优先级正确；离线用内置快照 | 单元、CLI | M2-1 | L | 未开始 |
| M2-5 | 游戏信息：MetadataService、steam-local/steam-store、IGDB/SGDB（模拟）、图片缓存 WebP、字段锁定 | 离线不阻塞；缓存 30 天 | 单元 | M2-2 | L | 未开始 |
| M2-6 | 识别率验收：本机只读扫描报告 | 达标或列出差距由用户决定 | 手工验收 | M2-2–M2-5 | S | 未开始 |
| M3-1 | Tauri 命令层、前端骨架（路由、中英文、三态主题、Logo）、首次运行向导 | 向导完成后进入游戏库 | Vitest | M2 | M | 未开始 |
| M3-2 | 游戏库页：逐游戏推送、徽标、筛选、勾选、备份方案、进度与取消 | 200 个模拟游戏流畅 | Vitest | M3-1 | L | 未开始 |
| M3-3 | 备份确认弹窗：逐游戏备注、统一备注、变化文件、同时打标签 | 备份后状态刷新 | Vitest | M3-2 | M | 未开始 |
| M3-4 | 游戏详情/历史：分支图时间线、文件与差异、编辑备注、标签/分支、搜索 | 与 CLI log 一致 | Vitest | M3-1 | L | 未开始 |
| M3-5 | 恢复 UI（预览、确认、保护快照、撤销提示、云存档提示）与设置页 | 恢复/撤销可用 | Vitest | M3-4 | M | 未开始 |
| M3-6 | NSIS 安装包、图标、P0 验收 | 安装、卸载、升级正常 | 手工验收 | M3-1–M3-5 | S | 未开始 |
| M4-1 | Ubisoft、EA、Xbox/MS Store（wgs 整体）、Battle.net 适配器 | 伪造目录测试通过 | 单元 | M2-1 | L | 未开始 |
| M4-2 | 游戏运行保护完善：运行中警告或阻止恢复 | 模拟进程测试通过 | 单元 | M1-6 | M | 未开始 |
| M4-3 | 托盘与自动备份（游戏退出、定时；运行中不做重 IO） | 退出游戏后自动生成快照 | 单元 | M3 | L | 未开始 |
| M4-4 | 保留策略、快照压缩、标记-清除 GC、packs | 标签不被清理；GC 后 fsck 通过 | 单元、CLI | M1 | L | 未开始 |
| M4-5 | 性能与资源基准测试 | 达到 10.1 指标 | 基准 | M4-1–M4-4 | M | 未开始 |
| M5-1 | 单游戏历史导入/导出 | 新仓库导入后历史一致 | 单元、CLI | M1 | M | 未开始 |
| M5-2 | 仓库 AES-256-GCM 可选加密 | 加密仓库读写、fsck 正常 | 单元 | M1 | L | 未开始 |
| M5-3 | VSS 读取锁定文件 | 锁定文件可备份 | 单元 | M1 | M | 未开始 |
| M5-4 | Linux / Steam Deck 支持 | Linux 上测试通过 | 全量 | M4 | L | 未开始 |

## 已确认的待确认点

| 问题 | 结论 | 确认日期 |
|---|---|---|
| 技术栈 | 按文档：Rust + Tauri 2 + React/TS | 2026-10-08 |
| 验收标准 | 10.1 非功能指标 + 10.4 里程碑验收 + P0 全部交付 | 2026-10-08 |
| Q1 CLI JSON 参数 | `--json` 与 `--output json` 都支持 | 2026-10-09 |
| Q2 安装包格式 | NSIS（按用户安装） | 2026-10-09 |
| Q3 Ludusavi 规则库获取 | 安装包内置快照，启动时联网增量更新 | 2026-10-09 |
| Q4 Rust / pnpm / MSVC 安装 | 由 Claude 安装 | 2026-10-09 |
| Q5 M2 真机验收 | 本机（已装 Steam），扫描只读 | 2026-10-09 |
| Q6 IGDB / SteamGridDB Key | 没有，用模拟数据测试 | 2026-10-09 |
| E2 远程 API | 不开发 | 2026-10-09 |
| R2-Q1 托盘形态 | 同一个程序，关闭窗口时销毁 WebView，只留托盘 | 2026-10-09 |
| R2-Q2 便携模式 | 支持（`portable` 标记文件） | 2026-10-09 |
| R2-Q3 进程检测 | M1 做简化版（警告），M4 完善 | 2026-10-09 |
| R2-Q4 应用配置位置 | `%APPDATA%\Bonfire\config.toml` | 2026-10-09 |

## T 测试

- T1 测试框架：Rust 用 `cargo test`（CLI 测试用 `assert_cmd` 执行真实二进制）；前端用 Vitest【用户修改：选择“帮我选择”，对话确认】
- T2 测试范围：单元、CLI、文档（无 API 测试）【建议：不开发远程 API】
- T3 一键测试脚本：`test.bat`【默认】
- T4 在线测试：不适用（无远程 API）
- 测试规则详见 `tests/README.md`。

## L Linux 开发与测试环境

- L1 Linux 环境：不使用（**未经 Linux 实测**，只在代码层面保持跨平台写法；M5 Linux 支持时重新接入）【建议：首版仅 Windows】

## 目录结构

见 `README.md`「项目结构」，两者保持一致。

## 不适用的维度

- C2 报告语言：项目不生成报告。
- E2 远程 API 及 E2-auth、E2-prefix、T4 在线测试、K2 远程 API 调用 skill：不开发远程 API。
- L2–L4 Linux 环境细项：不使用 Linux 环境。
- F3 Docker 部署（初选为不需要）：Windows 桌面 GUI 程序，不适合 Docker。

## 与技能默认约定的差异（技术栈为 Rust + Tauri）

| 技能约定 | 本项目做法 | 原因 |
|---|---|---|
| `src/<包名>/` Python 布局、`pyproject.toml`、uv | Cargo workspace（`crates/`）+ `app/`（Tauri + 前端） | 技术栈为 Rust + Tauri |
| 测试全部集中在 `tests/` | 用例数据、输入、CLI 与文档测试集中在 `tests/`（作为 workspace 成员 crate `bonfire-tests`）；核心库的单元测试按 Rust 惯例写在模块内 `#[cfg(test)]`；前端测试与组件同目录 `*.test.tsx` | 单元测试需要访问私有项；Vitest 惯例 |
| `scripts/build_icons.py` 导出图标 | 用 `pnpm tauri icon assets/brand/logo.svg` 生成全部尺寸图标 | 不引入 Python 依赖，Tauri 自带 |
| `start.sh` / `test.sh` | 首版只提供 `.bat`；M5 Linux 支持时补齐 `.sh` | 首版仅 Windows |
| `scripts/gen_docs.py` 生成文档示例 | 文档测试自带生成：`test.bat --bless` 改写 `docs/cli.md` 示例与 `config/config.example.toml` | 不引入 Python 依赖 |
| 文案放在 `src/<包名>/i18n/locales/` | `crates/bonfire-app/locales/*.json`，CLI 内嵌、前端直接导入同一份文件 | CLI 与 GUI 共用文案 |
| `config/config.toml` 在项目目录 | 运行时配置在 `%APPDATA%\Bonfire\`（便携模式在程序目录旁） | 桌面应用惯例（R2-Q4） |

## F 收尾辅助

（开发完成后第三轮确认。第一轮初选：F1 打包迁移脚本——需要，默认不包含数据/配置/日志；F2 迁移与部署——在线部署；F3 Docker——不需要。）

## K 项目专属 skill

（开发完成后第三轮确认。第一轮初选：K1 CLI 调用 skill、K3 测试 skill、K4 项目更新 skill 生成；K2 不适用。）
