# 测试说明

## 目录

```
tests/
├── README.md            本说明
├── Cargo.toml           测试 crate bonfire-tests（workspace 成员，只含测试）
├── inputs/              测试输入（入库，只读）
│   ├── README.md        每个输入文件的用途说明
│   ├── saves/           伪造的游戏存档目录
│   ├── downloads/       伪造的网上下载存档（嵌套文件夹、附带说明，“导入下载的存档”用）
│   └── platforms/       伪造的平台安装目录（Steam/Epic/GOG…，M2 起）
├── cases/
│   └── cli_cases.toml   CLI 用例，同时生成 docs/cli.md 的示例
├── cli/                 CLI 测试（执行真实的 bonfire 二进制）
├── docs/                文档测试
├── bench/               性能基准（文档 10.1 指标，release 构建手动运行）
├── installer/           安装包验收脚本 check-installer.ps1（构建安装包后手动运行）
└── output/              运行时产生的文件（不入库）
```

- 核心库单元测试按 Rust 惯例写在各模块的 `#[cfg(test)] mod tests` 中。
- 前端测试与组件同目录，命名 `*.test.ts(x)`，用 Vitest 运行。

## 规则

- **输入只读**：`tests/inputs/` 下的文件在测试中不修改；每个测试把需要的输入复制到临时目录后再操作，产生的文件写到临时目录或 `tests/output/`。
- **绝不触碰真实数据**：测试中的仓库、配置、存档目录一律使用临时目录（`BONFIRE_CONFIG`、`--repo` 指向临时位置），不读写 `%APPDATA%\Bonfire`、`%LOCALAPPDATA%\Bonfire` 和真实游戏存档。
- **体积小**：单个输入文件不超过 1 MB；大文件（FastCDC、吞吐测试）用固定种子在测试中生成。
- **三类情况**：正常输入、边界输入（空目录、单个文件、0 字节文件、超长路径、只读文件）、错误输入（不存在的路径、损坏的对象、错误的引用）。
- **多语言文字**：至少一份输入的文件名和备注包含中文与带重音的拉丁字符（如 ã、ç、é）。
- **结果确定**：时间、随机数在测试中固定。
- **用例覆盖**：每个 CLI 命令至少一条正常用例和一条错误用例（检查退出码、`--json` 输出字段、错误时 stderr 有提示）。至少一条用例分别以 `--lang zh-CN` 和 `--lang en` 运行。
- **崩溃一致性**：通过环境变量 `BONFIRE_FAULT_INJECT=<步骤>` 让子进程在写入的指定步骤中止，再验证 fsck 无错误、已有快照完好。

## 文档测试

| 检查项 | 方法 |
|---|---|
| CLI 文档示例 | `docs/cli.md` 示例部分与由 `cli_cases.toml` 重新生成的结果一致 |
| 目录结构 | `README.md`「项目结构」中的目录与实际目录一致（忽略不入库的目录） |
| 多语言完整 | 中英文文案文件的键集合一致 |
| 链接有效 | 文档中指向项目内文件的相对链接都存在 |

## 运行

| 命令 | 作用 |
|---|---|
| `test.bat` | 运行全部测试（cargo test --workspace + Vitest） |
| `test.bat --unit` / `--cli` / `--docs` / `--ui` | 只运行某一类 |
| `test.bat --bless` | 重新生成 `docs/cli.md` 示例与 `config/config.example.toml`，再运行文档测试 |

- 各类输出写入 `tests/output/<类别>.log`，汇总写入 `tests/output/report.txt`。
- 结束时输出各类通过 / 失败数量；有失败时退出码非 0。双击运行时失败会 `pause`；脚本或 Claude 调用时设置 `BONFIRE_NO_PAUSE=1` 跳过。

## Linux

未使用 Linux 环境（L1），首版未经 Linux 实测；M5 支持 Linux 时补充。

## 性能基准

```bat
cargo test -p bonfire-tests --release --test bench -- --ignored --nocapture --test-threads 1
```

在临时目录中按固定种子生成数据（200 款游戏共 1000 个文件、512 MB 与 256 MB 的不可压缩文件），测量增量扫描、全量校验、备份吞吐、未变化文件的新增存储与快照元数据大小，release 构建中不达标即失败；debug 构建只打印结果。不随 `test.bat` 运行。托盘常驻内存需要运行桌面程序，方法与结果见 `docs/acceptance/M4-acceptance.md`。

## 安装包测试

构建安装包（`build.bat`）后运行：

```bat
powershell -NoProfile -ExecutionPolicy Bypass -File tests\installer\check-installer.ps1
```

脚本把安装包静默安装到临时目录，依次检查：安装成功、卸载登记与开始菜单快捷方式、桌面程序为图形界面而命令行为控制台程序、命令行能读到内置规则库并在安装目录的 `data` 下建仓库、覆盖安装（升级）后数据仍在、卸载后程序与登记清除而 `data` 保留。输出 JSON，`passed` 为 `true` 时退出码 0。不启动桌面程序；本机已安装篝火时不运行（避免卸载测试清掉它的登记）。
