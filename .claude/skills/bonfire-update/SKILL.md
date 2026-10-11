---
name: bonfire-update
description: 篝火（Bonfire）的项目更新流程：出现新需求、任务文档更新或代码改动后，找出变更、分析需要同步更新的部分（核心库、CLI、界面、文案、配置、测试、文档、安装包、项目专属 skill），询问版本号，生成更新记录与汇总，必要时做需求盘点。用户说“任务文档更新了”“加一个需求”“我改了代码，帮我同步一下”“准备发新版本”“汇总一下更新记录”“盘点一下需求”“还有哪些功能没做完”“生成最终开发文档”，或 "prepare a new release", "sync docs with my changes", "summarize the changelog", "what requirements are still open" 时使用。
---

# 篝火项目更新

- “汇总一下更新记录”：直接按 `references/changelog-rules.md` 第 2 节“版本汇总”执行，不走下面的流程。
- 盘点类说法（“盘点一下需求”“还有哪些功能没做完”“生成最终开发文档”）：直接按 `references/review-rules.md` 执行。

## 流程

```
1 找出变更 → 2 影响分析 → 3 项目专属 skill 检测 → 4 询问版本号
→ 5 生成更新清单并确认 → 6 按顺序执行 → 7 测试与记录 → 8 需求盘点（仅主版本更新）
```

### 1. 找出变更

基准是 `docs/project-spec.md` 开头的 `last_synced_commit`（版本标签，如 `v0.1.0`）：

- 需求：`git diff <基准> -- DevelopDocs/`（`project-config/`、`review/` 中是选项与盘点记录，不算需求）。`DevelopDocs/` 出现新文件时先请用户确认用途，再登记到项目规范“开发文档”一节。
- 代码：`git diff --stat <基准>` 与未提交的改动。
- 新需求按项目规范“需求清单”规则编号（`REQ-` 接着往下排），询问放进本版本还是推迟。需求推迟或取消必须经用户同意。

### 2. 影响分析

读取 `references/impact-matrix.md` 第 1 节，逐类判断需要检查和更新的文件。

### 3. 项目专属 skill 检测

按 `references/impact-matrix.md` 第 2 节，对 `bonfire-cli`、`bonfire-test`、`bonfire-update` 逐个给出结论：“需要更新”（原因与改动）或“无需更新”，不能省略。

### 4. 询问版本号

- 当前版本在 `Cargo.toml` 的 `[workspace.package] version`（与 `app/package.json` 的 `version` 一致；Tauri 与各 crate 继承 workspace 版本）。
- 规则（语义化版本，主版本为 0 时）：不兼容改动或新增功能升次版本（`0.1.3 → 0.2.0`）；修复、文档、重构升修订号（`0.1.3 → 0.1.4`）。主版本 ≥ 1 时不兼容改动升主版本。何时发布 `1.0.0` 由用户决定，不主动建议。
- 给出建议值和理由，让用户选择“采用建议值 / 自填”；自填须为 `主.次.修订` 且大于当前版本。

### 5. 生成更新清单并确认

清单顶部写：本次版本号 `<当前> → <新版本>`；各项目专属 skill 的检测结论（每个一行）。正文每项：文件、原因、具体改动、是否不兼容。用户确认（可删减）后再执行。

### 6. 按顺序执行

1. 项目规范：需求清单、里程碑与开发阶段、能力清单、开发记录（新增一条，写明决策编号）
2. 核心逻辑：`crates/bonfire-core`、`crates/bonfire-discovery`、`crates/bonfire-app`
3. CLI（`crates/bonfire-cli`）与 Tauri 命令层（`app/src-tauri`）、前端（`app/src`）
4. 配置定义 `crates/bonfire-app/src/config.rs`、首次运行向导、文案 `crates/bonfire-app/locales/{zh-CN,en}.json`
5. 测试输入与用例数据（`tests/inputs/`、`tests/cases/cli_cases.toml`）、单元测试、前端测试
6. `test.bat --bless` 重新生成 `docs/cli.md` 示例与 `config/config.example.toml`，再更新其余文档（`docs/cli.md` 命令表、`tests/README.md`）
7. 脚本：`start.bat`、`test.bat`、`build.bat`、`cli.bat`；安装包配置 `app/src-tauri/tauri.bundle.json`
8. `README.md`（功能、项目结构）
9. 按检测结论更新项目专属 skill（含 `bonfire-test` 的功能清单），`bonfire-update` 自身放最后
10. 版本号（`Cargo.toml` 与 `app/package.json` 两处）、更新记录、`CHANGELOG.md`：按 `references/changelog-rules.md` 生成，测试情况先写“待测试”
11. 需要用户确认的选择写入决策记录：编写 JSON 后运行 `python DevelopDocs/project-config/decisions.py . <json文件>`，并打开 `DevelopDocs/project-config/decisions.html`

### 7. 测试与记录

1. 询问：测试本次更新的内容（建议，调用 `bonfire-test`，以本次涉及的功能编号预选）/ 全量测试 / 不测试。
2. 结果写入本次更新记录，重新生成所在次版本的 `SUMMARY.md`；不测试时如实记为“未测试”。
3. 失败时汇报失败的功能编号，询问修复后重测，还是如实记为“测试未通过”后结束（此时不更新 `last_synced_commit`）。
4. 通过或用户明确不测试后：把 `last_synced_commit` 设为 `v<新版本>`；询问是否提交并打标签 `v<新版本>`。暂不提交时提醒：提交时必须打同名标签。
5. 需要安装包时运行 `build.bat`（PowerShell：`$env:BONFIRE_NO_PAUSE="1"; cmd /c ".\build.bat --offline"`），产物在 `target\release\bundle\nsis\`。
6. 汇报：版本号变化、改动文件、各 skill 更新情况、测试结果、更新记录位置。

### 8. 需求盘点

主版本变化（如 `0.9.2 → 1.0.0`）时，在第 7 步之后按 `references/review-rules.md` 盘点。其他版本不自动执行。

## 约束

- 开发与测试绝不写入真实游戏存档目录；不操作用户安装的篝火（`%LOCALAPPDATA%\Bonfire` 或用户选择的安装目录）；界面检查用隔离实例（独立的 target 目录、`BONFIRE_HOME`、程序标识）。
- 界面改动先按 frontend-design 出方案再写代码。
- 与用户交流、文档与界面文案默认中文。
