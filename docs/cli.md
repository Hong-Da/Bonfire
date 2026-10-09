# CLI 调用文档

命令名 `bonfire`。开发时可在项目根目录用 `cli.bat <参数>` 调用（自动构建并切换 UTF-8 代码页）。

> 子命令随 M1 开发逐步加入，见 [项目规范](project-spec.md)「已确认的能力清单」。

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
- `json` 面向程序：结构固定，不随语言变化；结果写 stdout，错误写 stderr，格式为 `{"error": {"code": <退出码>, "message": "..."}}`。
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
bonfire --no-such-flag
```

退出码：2

### 配置文件格式错误（JSON 输出）

```bat
bonfire --config tests/inputs/config/bad-syntax.toml --json
```

退出码：3

<!-- examples:end -->
