# 测试输入

只读，测试时复制到临时目录使用。

| 文件 | 用途 | 使用者 |
|---|---|---|
| `config/bad-syntax.toml` | TOML 语法错误的配置文件 | CLI 用例 `config-bad-syntax-*` |
| `config/bad-value.toml` | 语法正确但取值无效（日志级别） | CLI 用例 `config-bad-value` |
| `config/zh-paths.toml` | 含中文与 ã ç é 的仓库路径，检查编码 | CLI 用例 `config-unicode-ok` |
