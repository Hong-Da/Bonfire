<p align="center"><img src="assets/brand/logo.svg" width="96"></p>

# 篝火 Bonfire

Windows 桌面端游戏存档备份工具：自动扫描各平台的游戏存档，判断哪些有更新，按 Git 式快照保存完整历史，随时恢复到任意版本。

> 开发中：M1（核心库 + 命令行）、M2（存档发现）、M3（桌面应用与安装包）已完成；M4（其余平台、历史清理等）进行中。

## 代码仓库

- Gitee：https://gitee.com/Hong_Xiao_Feng/bonfire.git
- GitHub：https://github.com/Hong-Da/Bonfire.git

## 功能

- **快照式备份**：每次备份生成一个快照，可写备注；内容寻址、分块去重，未变化的文件不重复占用空间。
- **更新检测**：按文件元数据快速判断哪些游戏有更新，必要时校验内容哈希。
- **版本历史**：时间线、快照差异、分支（多周目）与标签（永久保留的存档点），备注可搜索、可事后修改。
- **安全恢复**：恢复前预览差异、自动生成保护快照、失败自动回滚，恢复后可一键撤销；游戏运行时阻止恢复。
- **可靠**：任何时刻中断都不会损坏已有快照，`fsck` 可校验整个仓库。
- **自动识别**：扫描 Steam、Epic、GOG，按 Ludusavi 社区规则库（2 万多款游戏）定位存档，并获取中文名、封面、游玩时长等信息；规则库没有的游戏用启发式探测，确认后转为自定义规则。

## 安装

运行 `build.bat` 生成安装包 `target\release\bundle\nsis\Bonfire_<版本>_x64-setup.exe`（`build.bat --offline` 不下载规则库，使用已有快照）。安装包：

- 按当前用户安装（不需要管理员权限），默认装在 `%LOCALAPPDATA%\Bonfire`，开始菜单有“Bonfire”快捷方式；安装界面可选中文或英文。
- 数据（配置、备份仓库、日志、缓存）都在 `<安装目录>\data`。**卸载不会删除 `data`**，重新安装或升级后备份历史仍在。
- 命令行在 `<安装目录>\cli\bonfire.exe`，与桌面应用共用同一份数据；把 `<安装目录>\cli` 加入 PATH 后即可直接使用 `bonfire`。
- 内置 Ludusavi 规则库快照，首次启动不联网也能识别游戏，联网时自动增量更新。

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
├── cli.bat                     CLI 包装脚本
├── test.bat                    一键测试
├── config/
│   └── config.example.toml     应用配置示例（实际配置在程序目录的 data\ 中）
├── crates/
│   ├── bonfire-core/           引擎·版本存储：对象库、refs、索引、变更检测、fsck、GC
│   ├── bonfire-discovery/      引擎·存档发现：平台适配器、规则库、游戏信息提供者、截图来源
│   ├── bonfire-app/            应用服务层：扫描/备份/恢复/历史/清理、导出导入、截图相册、配置、日志、i18n
│   └── bonfire-cli/            命令行 bonfire
├── app/                        桌面应用（Tauri 2）
│   ├── package.json            前端依赖（pnpm）
│   ├── src/                    React + TypeScript 前端（页面、组件、i18n、主题）
│   └── src-tauri/              Tauri 壳：命令层、托盘与自动备份、图标
├── assets/
│   └── brand/                  Logo 源文件
├── DevelopDocs/                开发文档：任务文档与相关文件（只读）
│   └── project-config/         选项页面与用户的选择（history/ 不入库）
├── docs/
│   ├── project-spec.md         项目规范
│   ├── cli.md                  CLI 调用文档
│   └── acceptance/             里程碑验收报告
├── tests/                      测试：输入、用例数据、CLI 与文档测试（见 tests/README.md）
├── target/                     Rust 构建输出（不入库）
└── dist/                       打包结果（不入库）
```

## 测试

运行 `test.bat`，详见 [tests/README.md](tests/README.md)。
