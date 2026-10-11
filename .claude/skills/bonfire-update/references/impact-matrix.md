# 影响分析表与 skill 检测表

## 1. 影响分析表

| 变更类型 | 需要检查和更新的部分 |
|---|---|
| 新增或修改业务能力 | `crates/bonfire-app`（服务层）与引擎 crate（`bonfire-core`、`bonfire-discovery`）；能力清单（项目规范 E 组）；CLI `crates/bonfire-cli/src/{cli.rs,commands.rs}`；Tauri 命令 `app/src-tauri/src/commands.rs` 与 `main.rs` 注册；前端 `app/src/api.ts`、`types.ts`、页面与组件；`tests/cases/cli_cases.toml`；`docs/cli.md` |
| 修改 CLI 参数或输出 | `cli.rs`（参数）、`commands.rs`（输出）、参数说明文案 `cli.arg.<命令>.<参数>`（`every_command_and_arg_is_documented` 检查）、`cli_cases.toml`、`docs/cli.md`；退出码变化时同时更新 `docs/project-spec.md` E1 与 `bonfire-cli` skill 的退出码表 |
| 修改 Tauri 命令或返回结构 | `app/src-tauri/src/commands.rs`、`main.rs` 的 `invoke_handler`、`app/src/api.ts`、`app/src/types.ts`、`app/src/test-utils.tsx` 的模拟 API |
| 新增或修改配置项 | `crates/bonfire-app/src/config.rs`（结构、默认值、校验、`example_toml`）、`test.bat --bless` 重新生成 `config/config.example.toml`、设置页 `app/src/pages/Settings.tsx`、`app/src/types.ts` 的 `Config`、首次运行向导 `Wizard.tsx`（需要时） |
| 新增界面或命令行文字 | `crates/bonfire-app/locales/zh-CN.json` 与 `en.json` 两份都加；动态拼接的键登记到 `app/src/i18n.test.ts` 的 `DYNAMIC` 表 |
| 新增依赖 | 根 `Cargo.toml` 的 `[workspace.dependencies]` 与对应 crate 的 `Cargo.toml`、`Cargo.lock`；前端 `app/package.json` 与 `pnpm-lock.yaml` |
| 新增、移动、删除目录或入口文件 | `README.md`“项目结构”、`.gitignore`、`build.bat`、安装包资源 `app/src-tauri/tauri.bundle.json` |
| 修改仓库格式或索引结构 | `bonfire-core` 的 `repo.rs`（`FORMAT_VERSION`）或 `index.rs`（`SCHEMA_VERSION` 与迁移列）；旧仓库兼容性测试；`fsck`、`reindex` |
| 修改启动、安装或打包 | `start.bat`、`build.bat`、`tauri.bundle.json`、`tests/installer/check-installer.ps1`、`README.md`“安装” |
| 需要用户确认的设计选择 | 决策记录：JSON → `python DevelopDocs/project-config/decisions.py . <json>`，打开 `decisions.html`；项目规范“开发记录”注明决策编号 |
| 新增、修改、推迟或取消需求 | 项目规范的需求清单、里程碑与开发阶段 |
| 任何变更 | 测试（输入、用例、单元测试、前端测试）、版本号（`Cargo.toml` 与 `app/package.json`）、更新记录、`CHANGELOG.md` |

## 2. 项目专属 skill 检测表

| skill | 需要更新的情况 | 更新内容 |
|---|---|---|
| `bonfire-cli` | CLI 命令或参数增删改；JSON 输出结构变化；退出码变化；调用方式或前提检查变化（如加密仓库、安装路径） | 常用任务速查、结果解析与退出码表、前提检查、安全约束 |
| `bonfire-test` | 功能增删、改名、拆分、合并；测试命令、脚本、报告位置变化；新增测试文件 | `references/features/`：新增功能追加条目（没有合适分类时提议新分类），改名等修改条目并注明版本，删除标记“已移除（版本）”；运行方式变化时改 `SKILL.md` |
| `bonfire-update` | 影响分析表需要新增变更类型；执行顺序变化；版本号位置或更新记录规则变化；项目专属 skill 新增或移除 | 本文件、`changelog-rules.md`、`SKILL.md` 中的流程 |

规则：每个 skill 单独给出结论；只是 `docs/cli.md`、`tests/README.md` 变化而 skill 说明仍然成立时判定为“无需更新”；更新 skill 时同步补充 `description` 中的触发说法。
