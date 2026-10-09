# 篝火 Bonfire

开发文档：DevelopDocs/（用户编写，只读；任务文档为 DevelopDocs/游戏存档备份软件 · 开发设计文档.md，需求以此为准）
选项记录：DevelopDocs/project-config/config.json（用户的选择，修改时重新打开选项页面）
项目规范：docs/project-spec.md（实现以此为准，开发按其中的里程碑与开发阶段进行）
目录结构：见 README.md「项目结构」，新增或移动目录时同步更新
测试规则：见 tests/README.md，每完成一项功能就补上对应测试

## 要点

- 技术栈：Rust（Cargo workspace，`crates/`）+ Tauri 2 + React/TypeScript（`app/`，pnpm）。
- 业务逻辑只写在 `bonfire-app` / 引擎 crate 中，CLI 与 Tauri 命令层只做参数解析和输出。
- 扫描与备份对存档目录只读；只有恢复服务写入存档目录，且必须先生成保护快照。
- 开发与测试时绝不写入真实游戏存档目录；本机 Steam 只做只读扫描。
- 文档、注释、界面文案默认中文；界面支持中文 / English。
