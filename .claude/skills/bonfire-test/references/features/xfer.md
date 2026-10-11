# XFER 导入导出

存档文件、篝火包、zip、下载的存档。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| XFER-01 | 导出存档文件 | 当前存档或某个快照导出为 zip / 文件夹 | `crates/bonfire-app/src/transfer.rs` 测试；CLI 用例 `export-snapshot-zip`、`export-current-folder`、`export-into-save-dir`、`export-no-target` | 0.1.0 | 有效 |
| XFER-02 | 导入存档文件 | 作为新快照或新路线 | CLI 用例 `import-files-new-branch`、`import-files-identical`、`import-unknown-game` | 0.1.0 | 有效 |
| XFER-03 | 篝火包 | 历史包、合集包、zip、内容校验 | `crates/bonfire-app/src/transfer.rs`、`crates/bonfire-app/src/integrity.rs` 测试；CLI 用例 `bundle-export-empty`、`bundle-export-all`、`bundle-import-same`、`bundle-export-collection`、`bundle-export-zip`、`bundle-export-zip-scope`、`bundle-export-collection-scope`、`bundle-import-new-game`、`bundle-import-only-missing`、`bundle-import-not-bundle`、`bundle-export-shots-only`、`bundle-export-no-saves-needs-shots`、`bundle-import-no-shots`、`bundle-import-conflicting-parts`；`app/src/collection.test.tsx`、`app/src/transfer.test.tsx` | 0.1.0 | 有效 |
| XFER-04 | 导入网上下载的存档 | zip / 7z / rar / 文件夹，自动对应存档结构 | `crates/bonfire-app/src/downloads.rs` 测试；CLI 用例 `import-download`、`import-download-7z`、`import-download-7z-no-password`；`app/src/downloads.test.tsx` | 0.1.0 | 有效 |
