# MAINT 仓库维护与规则库

配置、校验、重建索引、规则库。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| MAINT-01 | 配置与全局参数 | 配置文件、环境变量、语言、错误输出 | `crates/bonfire-app/src/config.rs`、`crates/bonfire-app/src/i18n.rs` 测试；CLI 用例 `version-ok`、`help-ok`、`unknown-flag`、`config-bad-syntax-json`、`config-bad-syntax-zh`、`config-bad-syntax-en`、`config-bad-value`、`config-unicode-ok` | 0.1.0 | 有效 |
| MAINT-02 | 校验仓库 | 校验全部对象与引用 | `crates/bonfire-core/src/fsck.rs` 测试；CLI 用例 `fsck-ok`、`fsck-damaged` | 0.1.0 | 有效 |
| MAINT-03 | 重建索引 | 从仓库重建索引数据库 | `crates/bonfire-core/src/index.rs` 测试；CLI 用例 `reindex-ok`、`reindex-bad-repo` | 0.1.0 | 有效 |
| MAINT-04 | 规则库 | 内置快照、增量更新、用户规则 | `crates/bonfire-discovery/src/manifest.rs`、`crates/bonfire-discovery/src/http.rs` 测试；CLI 用例 `rules-info-ok`、`rules-update-offline`、`rules-info-bad-custom` | 0.1.0 | 有效 |
| MAINT-05 | 文档与性能 | 文档示例一致；性能基准 | `tests/docs/main.rs`；`tests/bench/main.rs`（手动，release） | 0.1.0 | 有效 |
