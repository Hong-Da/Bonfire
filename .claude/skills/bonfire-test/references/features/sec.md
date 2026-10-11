# SEC 加密与被占用文件

仓库加密、本机记住密码、读取被占用的存档。

| 编号 | 名称 | 说明 | 对应测试 | 引入版本 | 状态 |
|---|---|---|---|---|---|
| SEC-01 | 仓库加密 | AES-256-GCM + Argon2id，加密 / 取消加密 / 改密码 | `crates/bonfire-core/src/crypt.rs`、`crates/bonfire-app/src/security.rs` 测试；CLI 用例 `repo-encrypt`、`repo-encrypted-needs-password`、`repo-wrong-password`；`app/src/u10.test.tsx`（M5-2 仓库加密） | 0.1.0 | 有效 |
| SEC-02 | 被占用的存档 | 提示并推迟；可选卷影复制读取 | `crates/bonfire-app/src/vss.rs` 测试（真实卷影复制需要管理员确认，手动验收） | 0.1.0 | 有效 |
