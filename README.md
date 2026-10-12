<p align="center"><img src="assets/brand/logo.svg" width="96"></p>

# 篝火 Bonfire

Windows 桌面端游戏存档备份工具：自动扫描各平台的游戏存档，判断哪些有更新，按 Git 式快照保存完整历史，随时恢复到任意版本。

> 当前版本 0.1.0（[更新记录](CHANGELOG.md)）。只支持 Windows 10/11；Linux / Steam Deck 推迟到后续版本。

## 代码仓库

- Gitee：https://gitee.com/Hong_Xiao_Feng/bonfire.git
- GitHub：https://github.com/Hong-Da/Bonfire.git

## 功能

- **快照式备份**：每次备份生成一个快照，可写备注；内容寻址、分块去重，未变化的文件不重复占用空间。
- **更新检测**：按文件元数据快速判断哪些游戏有更新，必要时校验内容哈希。
- **版本历史**：时间线、快照差异、分支（多周目）与标签（永久保留的存档点），备注可搜索、可事后修改。
- **历史清理**：按保留策略（最近 N 个、每天 / 每周 1 个）清理多余的自动快照，带标签的快照永不清理；回收不再使用的空间。
- **安全恢复**：恢复前预览差异、自动生成保护快照、失败自动回滚，恢复后可一键撤销；游戏运行时阻止恢复（按安装目录或设置的游戏程序检测）。
- **自动备份**：定时、存档变化后、退出游戏后自动备份；游戏运行中不做自动备份，退出后再备份。
- **可选加密**：仓库可用密码加密（AES-256-GCM），放在网盘同步目录中也不怕被看到；可在本机记住密码。
- **上手简单**：首次运行向导几步完成设置（可从远程取回以前的备份和设置），功能导览介绍常用功能；都可以在设置中重新打开。
- **总览**：一眼看出存档在磁盘、篝火仓库、远程三处是否都在；同步或自动备份失败时导航、任务栏与托盘图标显示红点并弹出系统通知；仓库空间统计、本机操作记录；一键收集或写回全部游戏的截图。
- **同步备份**：把备份同步到 GitHub、Gitee、WebDAV、网盘、局域网共享或移动硬盘，可同时同步到多处；上传前检查文件与仓库大小，超限的文件排除并列出；可在每次备份后或定时自动同步，换电脑时从远程恢复。
- **可靠**：任何时刻中断都不会损坏已有快照，`fsck` 可校验整个仓库。
- **自动识别**：扫描 Steam、Epic、GOG、Ubisoft Connect、EA app、Xbox（Microsoft Store）、Battle.net，按 Ludusavi 社区规则库（2 万多款游戏）定位存档，并获取中文名、封面、游玩时长等信息；规则库没有的游戏用启发式探测，确认后转为自定义规则。

## 安装

运行 `build.bat` 生成安装包 `target\release\bundle\nsis\Bonfire_<版本>_x64-setup.exe`（`build.bat --offline` 不下载规则库，使用已有快照）。安装包：

- 按当前用户安装（不需要管理员权限），默认装在 `%LOCALAPPDATA%\Bonfire`，开始菜单有“Bonfire”快捷方式；安装界面可选中文或英文。
- 数据（配置、备份仓库、日志、缓存）都在 `<安装目录>\data`。卸载时会询问是否同时删除 `data`（默认保留，删除前再确认一次）；选择保留时，重新安装或升级后备份历史仍在。升级安装不会询问、也不删除。**已同步到 GitHub、Gitee、WebDAV、网盘或其他文件夹的远程备份不会被删除**；仓库位置改到 `data` 以外时，那里的仓库也不会被删除。
- 命令行在 `<安装目录>\cli\bonfire.exe`，与桌面应用共用同一份数据；把 `<安装目录>\cli` 加入 PATH 后即可直接使用 `bonfire`。
- 内置 Ludusavi 规则库快照，首次启动不联网也能识别游戏，联网时自动增量更新。

发布包与换电脑见 [docs/migration.md](docs/migration.md)：`pack.bat` 生成带版本号与运行环境的免安装包（默认，如 `Bonfire_<版本>_win-x64_portable.zip`）、安装包或迁移包（安装包 + 迁移说明 + 可选数据）。

## 使用方式

命令行（详见 [docs/cli.md](docs/cli.md)）：

```bat
bonfire rules update
bonfire scan
bonfire game add 空洞骑士 --path "%USERPROFILE%\AppData\LocalLow\Team Cherry\Hollow Knight"
bonfire status
bonfire backup 空洞骑士 -m "打完螳螂领主" --tag 螳螂
bonfire log 空洞骑士
bonfire restore 空洞骑士 HEAD~1
bonfire undo-restore 空洞骑士
```

## 快速开始

开发环境需要：Rust stable（MSVC）、Visual Studio Build Tools（C++ 生成工具 + Windows SDK）、Node.js 20+、pnpm、WebView2。`start.bat` 会检查并提示缺少的项；Build Tools 可以这样安装：

```bat
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.22621"
```

```bat
start.bat        :: 检查环境、安装依赖、构建并启动桌面应用；首次运行进入配置向导
cli.bat --help   :: 调用命令行 bonfire
test.bat         :: 运行全部测试
```

## 项目结构

```
bonfire/
├── README.md                   项目说明（含本目录结构）
├── CLAUDE.md                   给 Claude Code 的项目说明
├── Cargo.toml / Cargo.lock     Rust workspace 定义与锁定文件
├── rust-toolchain.toml         固定 Rust 稳定版
├── .gitignore / .gitattributes
├── start.bat                   一键运行
├── build.bat                   构建安装包（NSIS）
├── pack.bat                    打包：免安装包（默认）/ 安装包 / 迁移包，文件名带版本与运行环境 → dist\
├── cli.bat                     CLI 包装脚本
├── test.bat                    一键测试
├── CHANGELOG.md                更新记录总览
├── .claude/skills/             项目专属 skill：bonfire-cli、bonfire-test、bonfire-update
├── scripts/
│   └── pack.ps1                pack.bat 的实现
├── config/
│   └── config.example.toml     应用配置示例（实际配置在程序目录的 data\ 中）
├── crates/
│   ├── bonfire-core/           引擎·版本存储：对象库、refs、索引、变更检测、fsck、GC
│   ├── bonfire-discovery/      引擎·存档发现：平台适配器、规则库、游戏信息提供者、截图来源
│   ├── bonfire-app/            应用服务层：扫描/备份/恢复/历史/清理、导出导入、截图相册、同步备份、配置、日志、i18n
│   └── bonfire-cli/            命令行 bonfire
├── app/                        桌面应用（Tauri 2）
│   ├── package.json            前端依赖（pnpm）
│   ├── src/                    React + TypeScript 前端（页面、组件、i18n、主题）
│   └── src-tauri/              Tauri 壳：命令层、托盘与自动备份、图标
├── assets/
│   └── brand/                  Logo 源文件
├── DevelopDocs/                开发文档：任务文档与相关文件（只读）
│   ├── project-config/         选项页面、用户的选择与开发中的决定（history/ 不入库）
│   └── review/                 需求盘点（v0.1.0_2026-10-11/：已完成、新增、待开发、合并后的任务文档）
├── docs/
│   ├── project-spec.md         项目规范
│   ├── cli.md                  CLI 调用文档
│   ├── migration.md            安装、升级与迁移说明
│   ├── changelog/              各版本更新记录（v0/v0.1/v0.1.0.md…）
│   ├── design/                 界面设计方案（overview-2026-10-11/：总览页；wizard-2026-10-11/：首次运行向导与功能导览；ui-polish-2026-10-11/：界面优化改前改后对比）
│   └── acceptance/             里程碑验收报告
├── tests/                      测试：输入、用例数据、CLI 与文档测试（见 tests/README.md）
├── target/                     Rust 构建输出（不入库）
└── dist/                       打包结果（不入库）
```

## 项目专属 skill

在本项目中使用 Claude Code 时可用（`.claude/skills/`）：

- `bonfire-cli`：用命令行操作篝火（备份、恢复、查历史、导出、清理等）。
- `bonfire-test`：全量或按功能运行测试、解读失败、新增用例、验收核对。
- `bonfire-update`：需求或代码变更后判断要同步更新的部分，确定版本号，生成更新记录与需求盘点。

## 测试

运行 `test.bat`，详见 [tests/README.md](tests/README.md)。
