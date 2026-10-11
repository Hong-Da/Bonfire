---
name: bonfire-test
description: 运行和维护篝火（Bonfire）的测试：全量测试、按功能选择测试、只跑文档测试、性能基准，解读失败原因，新增测试用例，并对照任务文档做验收核对。用户说“跑一下测试”“测一下备份和恢复”“只测截图相关的”“加一个 CLI 用例”“测试为什么失败”“做一下验收检查”，或 "run the tests", "test the restore feature", "add a CLI test case", "why did the test fail" 时使用。
---

# 篝火测试

测试规则、目录与输入说明以 `tests/README.md` 为准；功能与测试的对应关系在 `references/features/`（先读 `_index.md`）。

## 1. 先询问测试方式

| 方式 | 命令 |
|---|---|
| 全量测试（建议） | `test.bat`（PowerShell 中 `$env:BONFIRE_NO_PAUSE="1"; cmd /c ".\test.bat"`），报告在 `tests\output\report.txt` |
| 按类别 | `test.bat --unit` / `--cli` / `--docs` / `--ui`（可组合） |
| 按功能选择 | 读 `references/features/_index.md`，让用户选分类（可多选，超过 4 个用编号列表），再列出所选分类的功能让用户勾选，按下面第 2 节只跑对应测试 |
| 仅文档测试 | `test.bat --docs` |
| 性能基准 | `cargo test -p bonfire-tests --release --test bench -- --ignored --nocapture --test-threads 1`（见 `tests/README.md`“性能基准”） |

没有 Linux 测试环境（Linux 支持已推迟）。

## 2. 按功能运行

把所选功能“对应测试”一栏汇总后分别运行：

- **Rust 单元测试**：按模块过滤，例如 `crates/bonfire-app/src/retention.rs` → `cargo test -p bonfire-app retention`；`crates/bonfire-core/src/gc.rs` → `cargo test -p bonfire-core gc`；`crates/bonfire-discovery/src/stores.rs` → `cargo test -p bonfire-discovery stores`。
- **CLI 用例**（`tests/cases/cli_cases.toml` 中的 `id`）：先 `cargo build -q -p bonfire-cli`，再 `cargo test -p bonfire-tests --test cli`。这条命令会跑全部用例（同一个测试函数），失败信息中以 `[用例 id]` 标出；汇报时只挑所选用例的结果。
- **崩溃一致性**：`cargo test -p bonfire-tests --test crash`（压力测试加 `-- --ignored`）。
- **前端（Vitest）**：在 `app/` 下 `pnpm vitest run src/<文件>.test.tsx`，可加 `-t "<describe 或 it 名称>"`。
- **文档测试**：`cargo test -p bonfire-tests --test docs`。

## 3. 解读结果

- 按功能编号汇报：每个编号通过 / 失败，失败时附测试名或用例 id 与关键报错。
- 常见原因：
  - CLI 用例失败但单元测试通过 → 先确认重新 `cargo build -p bonfire-cli`（用例执行的是已构建的二进制）。
  - 文档测试提示“不是最新” → 文案、CLI 用例或配置项改了，运行 `test.bat --bless` 重新生成 `docs/cli.md` 示例与 `config/config.example.toml`，检查差异后提交。
  - `i18n.test.ts` 失败 → 新文案只加了一种语言，或动态拼接的键没有登记到该文件的 `DYNAMIC` 表。
  - 偶发失败（时间、同一秒内的快照）→ 重跑一次确认，再查排序或时间依赖。
- 测试会生成临时目录，绝不使用真实存档目录、`%APPDATA%\Bonfire` 或安装版的 `data`。

## 4. 新增用例

- 按 `tests/README.md` 的规则：输入放 `tests/inputs/`（只读、单个文件 ≤ 1 MB），CLI 用例写进 `tests/cases/cli_cases.toml`（每个命令至少一条正常、一条错误用例；需要密码等环境变量时用 `env = { ... }`）；需要出现在文档中的加 `in_docs = true`，然后 `test.bat --bless`。
- 同步在 `references/features/<分类>.md` 中登记“对应测试”；用例不属于任何已有功能时，先问用户是新增功能条目还是归入已有功能（新增条目编号接着往下排，引入版本写当前版本）。

## 5. 验收检查

对照任务文档 `DevelopDocs/游戏存档备份软件 · 开发设计文档.md` 的功能需求（F1–F10）与非功能需求（10.1），结合 `docs/project-spec.md` 的需求清单状态和 `docs/acceptance/` 中的验收报告，逐条输出：需求、对应功能编号、测试结果、结论（通过 / 未验证 / 未通过 / 推迟）。
