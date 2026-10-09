# M2 存档发现 · 本机识别率验收报告

> 日期：2026-10-09　环境：开发机（Windows 11，Steam 两个库目录、1 个账号；未安装 Epic、GOG）
> 方式：`bonfire rules update` 下载 Ludusavi 规则库后，`bonfire scan --platform steam` 只读扫描；仓库位于临时目录，未写入任何游戏存档。

## 结论

| 指标 | 结果 | 要求 |
|---|---|---|
| 规则库 | 23,606 款游戏（精简后 718 KB，下载并转换 3 秒，加载 0.14 秒） | — |
| 已安装条目 | 20 个（其中 Wallpaper Engine 为软件，不计入游戏） | — |
| 匹配到存档规则 | 18 / 19 款游戏 = **94.7%** | ≥ 90%（文档 10.4） |
| 找到存档并入库 | 17 款 | — |
| 扫描耗时 | 0.53 秒（含规则加载） | 增量扫描 < 3 秒（文档 10.1） |
| 首次全量备份 | 17 款、372 个文件、约 20 MB，1.6 秒；去重后仓库 13 MB | — |

文档要求“50 款常见游戏”，本机实际安装 19 款游戏，按 Q5 的决定以本机实际为准；另有伪造平台目录的自动化测试覆盖 Steam、Epic、GOG 三个适配器。

## 明细

| 游戏 | 规则 | 存档 |
|---|---|---|
| Another Crab's Treasure、Atomic Heart、Biped、Black Myth: Wukong、Cuphead、Delta Force、Grand Theft Auto V Enhanced、Halo: MCC、Hollow Knight: Silksong、It Takes Two、Minishoot' Adventures、Monument Valley 3、Palworld、PUBG、Sifu、Split Fiction、Universe Sandbox | 命中 | 找到 |
| Stray | 命中 | 未找到（本机可能未游玩） |
| Biped 2 | 规则库暂无 | 未找到；启发式探测未命中 |
| Wallpaper Engine | 软件，不计入 | — |

## 游戏信息

联网抽查（Steam Store API + 本地 librarycache）：中文名、开发商、发行商、发行日期、类型、封面、横幅、图标、游玩时长、云存档标记均可获取；图片转为 WebP 并生成封面缩略图。
