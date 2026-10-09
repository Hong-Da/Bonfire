<p align="center"><img src="assets/brand/logo.svg" width="96"></p>

# 篝火 Bonfire

Windows 桌面端游戏存档备份工具：自动扫描各平台的游戏存档，判断哪些有更新，按 Git 式快照保存完整历史，随时恢复到任意版本。

> 开发中（M1 核心库）。功能、使用方式等章节随开发补充。

## 快速开始

开发环境需要：Rust stable（MSVC）、Visual Studio Build Tools（C++ 生成工具 + Windows SDK）、Node.js 20+、pnpm、WebView2。

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
├── cli.bat                     CLI 包装脚本
├── test.bat                    一键测试
├── config/
│   └── config.example.toml     应用配置示例（实际配置在 %APPDATA%\Bonfire\）
├── crates/
│   ├── bonfire-core/           引擎·版本存储：对象库、refs、索引、变更检测、fsck、GC
│   ├── bonfire-discovery/      引擎·存档发现：平台适配器、规则库、游戏信息提供者
│   ├── bonfire-app/            应用服务层：扫描/备份/恢复/历史/清理服务、配置、日志、i18n
│   └── bonfire-cli/            命令行 bonfire
├── app/                        桌面应用（Tauri 2）
│   ├── package.json            前端依赖（pnpm）
│   ├── src/                    React + TypeScript 前端（页面、组件、i18n、主题）
│   └── src-tauri/              Tauri 壳：命令层、托盘与自动备份、图标
├── assets/brand/               Logo 源文件
├── DevelopDocs/                开发文档：任务文档与相关文件（只读）
│   ├── project-config/         选项页面与用户的选择（history/ 不入库）
│   └── review/                 需求盘点结果（只新增，不覆盖）
├── docs/
│   ├── project-spec.md         项目规范
│   └── cli.md                  CLI 调用文档
├── scripts/                    辅助脚本：生成文档示例、准备规则库快照等
├── tests/                      测试：输入、用例数据、CLI 与文档测试（见 tests/README.md）
├── target/                     Rust 构建输出（不入库）
└── dist/                       打包结果（不入库）
```

## 测试

运行 `test.bat`，详见 [tests/README.md](tests/README.md)。
