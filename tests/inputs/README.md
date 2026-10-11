# 测试输入

只读，测试时复制到临时目录使用。

| 文件 | 用途 | 使用者 |
|---|---|---|
| `config/bad-syntax.toml` | TOML 语法错误的配置文件 | CLI 用例 `config-bad-syntax-*` |
| `config/bad-value.toml` | 语法正确但取值无效（日志级别） | CLI 用例 `config-bad-value` |
| `config/zh-paths.toml` | 含中文与 ã ç é 的仓库路径，检查编码 | CLI 用例 `config-unicode-ok` |
| `saves/basic/` | 伪造的存档目录：`slot1.sav`、含中文与 ã ç é 的子目录文件、`options.ini` | CLI 用例 `fixture = "basic"`、崩溃测试 |
| `repo-future/config.toml` | 格式版本高于本程序的仓库（打开即报错，不会写入） | CLI 用例 `scan-bad-repo`、`reindex-bad-repo` |
| `roots/steam-basic/` | 伪造的“本机”：`pc/` 下有 Steam（Hollow Knight 有规则、Indie Quest 无规则）、游玩时长、两款游戏的存档；`home/custom-rules.yaml` 为用户规则 | CLI 用例 `root = "steam-basic"` |
| `roots/bad-rules/` | 格式错误的用户规则文件 | CLI 用例 `rules-info-bad-custom` |

## downloads/

伪造的网上下载存档（“导入下载的存档”与 CLI `import --download` 用例）：

- `hk-allcollect/全收集存档/存档/slot1.sav`：套在两层文件夹里的存档文件，与 `saves/basic/slot1.sav` 同名，用来测试按文件名自动对应。
- `hk-allcollect/全收集存档/使用说明.txt`：附带的说明，导入时应被忽略。
