# GAME 游戏管理与发现

平台扫描、规则库匹配、手动添加、加入 / 忽略 / 移出游戏库、游戏信息。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| GAME-01 | 手动添加游戏 | 按存档目录或文件添加，可带配置文件与安装目录 | CLI 用例 `game-add-ok`、`game-add-missing-path`、`game-list-json`、`game-list-bad-format` | 0.1.0 | 有效 |
| GAME-02 | 发现本机游戏 | 扫描 Steam / Epic / GOG，按 Ludusavi 规则库定位存档，不自动加入 | `crates/bonfire-discovery/src/platforms.rs`、`crates/bonfire-discovery/src/resolve.rs`、`crates/bonfire-app/src/discover.rs` 测试；CLI 用例 `scan-ok`、`scan-discovers-steam`、`scan-json`、`scan-no-discover`、`scan-bad-repo` | 0.1.0 | 有效 |
| GAME-03 | 其余平台 | Ubisoft Connect、EA app、Battle.net（卸载程序登记）、Xbox / Microsoft Store（wgs 整体备份） | `crates/bonfire-discovery/src/stores.rs`、`crates/bonfire-discovery/src/resolve.rs` 测试（xbox / ubisoft） | 0.1.0 | 有效 |
| GAME-04 | 加入 / 忽略 / 确认存档位置 | 发现的游戏加入或忽略；启发式位置确认后备份 | CLI 用例 `game-include-ok`、`game-include-all-new`、`game-include-unknown`、`game-include-nothing`、`game-ignore-ok`、`scan-after-ignore`、`game-ignore-unknown`、`game-confirm-ok`、`game-confirm-unknown`、`game-include-mode`、`game-include-bad-mode` | 0.1.0 | 有效 |
| GAME-05 | 移出游戏库 | 手动添加的删除、扫描发现的改为忽略；可同时删除备份 | CLI 用例 `game-rm-needs-yes`、`game-rm-ok`、`game-rm-discovered-ignores`、`game-list-all`、`game-rm-delete-snapshots-protected`、`game-rm-delete-shots` | 0.1.0 | 有效 |
| GAME-06 | 游戏信息 | 中文名、封面、游玩时长等，可手动修改 | `crates/bonfire-discovery/src/metadata.rs`、`crates/bonfire-app/src/metadata.rs` 测试；CLI 用例 `info-ok`、`info-set-json`、`info-bad-field`、`info-unknown-game`、`backup-discovered-records-playtime` | 0.1.0 | 有效 |
| GAME-07 | 管理方式 | 存档和截图 / 只管理存档 / 只管理截图 | CLI 用例 `game-mode`、`game-mode-show`、`game-mode-bad-value`、`game-shots-only-on`、`game-shots-only`、`game-shots-only-bad-value` | 0.1.0 | 有效 |
