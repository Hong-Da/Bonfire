//! 仓库加密（文档 10.1 安全，M5-2）：AES-256-GCM。
//!
//! - 随机生成 32 字节的数据密钥，加密对象、备注与引用日志；数据密钥再用密码经 Argon2id 派生的密钥加密后存进
//!   `config.toml` 的 `[encryption]`。修改密码只需重新加密数据密钥，不用改动数据。
//! - 加密后的数据以 `BFE1` 开头，后跟 12 字节随机数与密文（含认证标签）；读取时按开头判断，加密中途中断时
//!   新旧格式混在一起也能读。
//! - 对象 ID 仍是明文内容的哈希（用于去重与变更检测），不泄露内容；分支、标签名与截图不加密。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// 加密数据的开头。
pub const MAGIC: &[u8; 4] = b"BFE1";
const NONCE: usize = 12;
/// 检验密码用的明文。
const CHECK: &[u8] = b"bonfire-key-check";

/// `config.toml` 中的 `[encryption]`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptionConfig {
    /// 目前只有 `argon2id`。
    pub kdf: String,
    /// Argon2id 参数：内存（KiB）、迭代次数、并行度。
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
    pub salt: String,
    /// 用密码派生的密钥加密后的数据密钥（十六进制）。
    pub wrapped_key: String,
    /// 用数据密钥加密的检验值，记住的密钥也能据此检验。
    pub check: String,
    /// 仓库标识：本机记住的密钥、移出仓库的索引按它存放。
    pub repo_id: String,
}

/// 数据密钥。
pub struct Cipher {
    aead: Aes256Gcm,
    key: [u8; 32],
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cipher(…)")
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("系统随机数可用");
    b
}

fn seal_with(aead: &Aes256Gcm, plain: &[u8]) -> Vec<u8> {
    let nonce = random::<NONCE>();
    let ct = aead
        .encrypt(Nonce::from_slice(&nonce), plain)
        .expect("AES-GCM 加密不会失败");
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn open_with(aead: &Aes256Gcm, data: &[u8]) -> Option<Vec<u8>> {
    let rest = data.strip_prefix(MAGIC.as_slice())?;
    if rest.len() < NONCE {
        return None;
    }
    let (nonce, ct) = rest.split_at(NONCE);
    aead.decrypt(Nonce::from_slice(nonce), ct).ok()
}

/// 数据是否为加密格式。
pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

impl Cipher {
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            aead: Aes256Gcm::new_from_slice(&key).expect("密钥长度为 32 字节"),
            key,
        }
    }

    /// 数据密钥（本机记住密码时由调用方加密保存）。
    pub fn key(&self) -> [u8; 32] {
        self.key
    }

    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        seal_with(&self.aead, plain)
    }

    /// 解密；`what` 只用于错误信息。
    pub fn open(&self, data: &[u8], what: &str) -> Result<Vec<u8>> {
        open_with(&self.aead, data).ok_or_else(|| {
            CoreError::BadRepoConfig(format!("{what} 解密失败（数据损坏或密钥不对）"))
        })
    }

    /// 检验密钥是否属于这个仓库。
    pub fn verify(&self, cfg: &EncryptionConfig) -> bool {
        hex::decode(&cfg.check)
            .ok()
            .and_then(|c| open_with(&self.aead, &c))
            .is_some_and(|p| p == CHECK)
    }
}

fn kdf(password: &str, cfg: &EncryptionConfig) -> Result<Aes256Gcm> {
    let salt = hex::decode(&cfg.salt).map_err(|e| CoreError::BadRepoConfig(e.to_string()))?;
    let params = argon2::Params::new(cfg.m_kib, cfg.t, cfg.p, Some(32))
        .map_err(|e| CoreError::BadRepoConfig(e.to_string()))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), &salt, &mut out)
        .map_err(|e| CoreError::BadRepoConfig(e.to_string()))?;
    Ok(Aes256Gcm::new_from_slice(&out).expect("密钥长度为 32 字节"))
}

/// 测试中用小参数加快密钥派生（真实使用时不要打开）。
pub static FAST_KDF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Argon2id 参数（OWASP 建议值：64 MiB、3 次、1 路）；测试中用小参数加快速度。
fn params() -> (u32, u32, u32) {
    let fast = FAST_KDF.load(std::sync::atomic::Ordering::Relaxed)
        || std::env::var_os("BONFIRE_TEST_KDF").is_some();
    if cfg!(test) || fast {
        (256, 1, 1)
    } else {
        (64 * 1024, 3, 1)
    }
}

/// 新的加密设置：随机数据密钥，用密码加密保存。
pub fn new_config(password: &str) -> Result<(EncryptionConfig, Cipher)> {
    let key = random::<32>();
    let (m, t, p) = params();
    let mut cfg = EncryptionConfig {
        kdf: "argon2id".into(),
        m_kib: m,
        t,
        p,
        salt: hex::encode(random::<16>()),
        wrapped_key: String::new(),
        check: String::new(),
        repo_id: hex::encode(random::<8>()),
    };
    let cipher = Cipher::from_key(key);
    cfg.check = hex::encode(cipher.seal(CHECK));
    cfg.wrapped_key = hex::encode(seal_with(&kdf(password, &cfg)?, &key));
    Ok((cfg, cipher))
}

/// 用密码取出数据密钥；密码不对时返回 [`CoreError::WrongPassword`]。
pub fn unlock(cfg: &EncryptionConfig, password: &str) -> Result<Cipher> {
    let wrapped =
        hex::decode(&cfg.wrapped_key).map_err(|e| CoreError::BadRepoConfig(e.to_string()))?;
    let key = open_with(&kdf(password, cfg)?, &wrapped).ok_or(CoreError::WrongPassword)?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| CoreError::BadRepoConfig("数据密钥长度不对".into()))?;
    Ok(Cipher::from_key(key))
}

/// 修改密码：数据密钥不变，换一个盐重新用新密码加密。
pub fn rewrap(
    cfg: &EncryptionConfig,
    cipher: &Cipher,
    new_password: &str,
) -> Result<EncryptionConfig> {
    let mut next = cfg.clone();
    next.salt = hex::encode(random::<16>());
    let (m, t, p) = params();
    next.m_kib = m;
    next.t = t;
    next.p = p;
    next.wrapped_key = hex::encode(seal_with(&kdf(new_password, &next)?, &cipher.key()));
    Ok(next)
}

/// 加密整个仓库：先写入加密设置（之后中断也能用密码读取），再把全部对象、引用日志与备注改为加密格式。
/// 返回重新打开并已解锁的仓库。
pub fn encrypt_repo(
    repo: &crate::Repo,
    lock: &crate::RepoLock,
    password: &str,
) -> Result<crate::Repo> {
    if repo.is_encrypted() {
        return Err(CoreError::BadRepoConfig("仓库已经加密".into()));
    }
    let (cfg, cipher) = new_config(password)?;
    repo.write_encryption(lock, Some(cfg))?;
    let r = crate::Repo::open(repo.root())?;
    r.unlock_with_key(cipher.key());
    reseal_all(&r, lock, true)?;
    Ok(r)
}

/// 取消加密：先把数据改回明文，最后删除加密设置。返回重新打开的仓库。
pub fn decrypt_repo(repo: &crate::Repo, lock: &crate::RepoLock) -> Result<crate::Repo> {
    if !repo.is_encrypted() {
        return Err(CoreError::BadRepoConfig("仓库没有加密".into()));
    }
    if !repo.is_unlocked() {
        return Err(CoreError::NeedPassword);
    }
    reseal_all(repo, lock, false)?;
    repo.write_encryption(lock, None)?;
    crate::Repo::open(repo.root())
}

/// 修改密码（数据密钥不变，数据不用改写）。返回重新打开并已解锁的仓库。
pub fn change_password(
    repo: &crate::Repo,
    lock: &crate::RepoLock,
    old: &str,
    new: &str,
) -> Result<crate::Repo> {
    let cfg = repo
        .config()
        .encryption
        .as_ref()
        .ok_or_else(|| CoreError::BadRepoConfig("仓库没有加密".into()))?;
    let cipher = unlock(cfg, old)?;
    let next = rewrap(cfg, &cipher, new)?;
    repo.write_encryption(lock, Some(next))?;
    let r = crate::Repo::open(repo.root())?;
    r.unlock_with_key(cipher.key());
    Ok(r)
}

fn reseal_all(repo: &crate::Repo, lock: &crate::RepoLock, encrypt: bool) -> Result<()> {
    use rayon::prelude::*;
    for kind in crate::object::Kind::ALL {
        let ids = repo.list_objects(kind)?;
        ids.par_iter()
            .try_for_each(|id| repo.reseal_object(lock, kind, id, encrypt).map(|_| ()))?;
    }
    repo.reseal_logs_and_notes(lock, encrypt)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_and_passwords() {
        let (cfg, c) = new_config("火堆 bonfire").unwrap();
        let sealed = c.seal(b"save data");
        assert!(is_sealed(&sealed));
        assert_ne!(c.seal(b"save data"), sealed, "每次随机数不同");
        assert_eq!(c.open(&sealed, "x").unwrap(), b"save data");
        assert!(c.verify(&cfg));

        let again = unlock(&cfg, "火堆 bonfire").unwrap();
        assert_eq!(again.open(&sealed, "x").unwrap(), b"save data");
        assert!(matches!(
            unlock(&cfg, "wrong"),
            Err(CoreError::WrongPassword)
        ));

        let next = rewrap(&cfg, &c, "新密码").unwrap();
        assert!(matches!(
            unlock(&next, "火堆 bonfire"),
            Err(CoreError::WrongPassword)
        ));
        assert_eq!(
            unlock(&next, "新密码").unwrap().key(),
            c.key(),
            "数据密钥不变"
        );

        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(c.open(&bad, "x").is_err(), "篡改后认证失败");
        assert!(!Cipher::from_key([7; 32]).verify(&cfg));
    }

    #[test]
    fn encrypt_and_decrypt_whole_repo() {
        use crate::object::{Kind, Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
        use crate::refs::RefName;
        let d = tempfile::tempdir().unwrap();
        let repo = crate::Repo::init(d.path()).unwrap();
        let lock = repo.lock().unwrap();
        let stats = repo
            .write_file(&b"secret save"[..], std::path::Path::new("x"))
            .unwrap();
        let tree = repo
            .put_tree(&Tree::new(vec![TreeEntry {
                path: "save/a.sav".into(),
                blob: stats.blob,
                size: stats.size,
                mtime_ns: 0,
                tag: crate::FileTag::Save,
            }]))
            .unwrap();
        let snap = repo
            .put_snapshot(&Snapshot {
                game: "g".into(),
                tree,
                parents: vec![],
                created_at: "2026-10-11T00:00:00+08:00".into(),
                trigger: Trigger::Manual,
                host: "pc".into(),
                comment: "打完 Boss".into(),
                stats: SnapshotStats::default(),
                playtime_min: None,
            })
            .unwrap();
        repo.update_ref(
            &lock,
            "g",
            &RefName::branch("main"),
            None,
            snap,
            "backup",
            "打完 Boss",
        )
        .unwrap();
        repo.set_note(&lock, &snap, "改过的备注").unwrap();

        let r = encrypt_repo(&repo, &lock, "pw").unwrap();
        let raw = std::fs::read(r.object_path(Kind::Snapshot, &snap)).unwrap();
        assert!(is_sealed(&raw), "对象文件已加密");
        let log = std::fs::read_to_string(d.path().join("logs/g.jsonl")).unwrap();
        assert!(
            log.starts_with("enc:") && !log.contains("Boss"),
            "引用日志已加密"
        );
        assert!(
            !std::fs::read_to_string(d.path().join("config.toml"))
                .unwrap()
                .contains("secret")
        );

        // 重新打开：没有密码读不了，密码不对报错，密码对了一切正常。
        let locked = crate::Repo::open(d.path()).unwrap();
        assert!(locked.is_encrypted() && !locked.is_unlocked());
        assert!(matches!(
            locked.get_snapshot(&snap),
            Err(CoreError::NeedPassword)
        ));
        assert!(matches!(
            locked.unlock("nope"),
            Err(CoreError::WrongPassword)
        ));
        locked.unlock("pw").unwrap();
        assert_eq!(locked.get_snapshot(&snap).unwrap().comment, "打完 Boss");
        assert_eq!(locked.note(&snap).unwrap().unwrap().comment, "改过的备注");
        assert_eq!(locked.reflog("g").unwrap().len(), 1);
        assert!(crate::fsck::fsck(&locked, true).unwrap().ok());
        // 新写入的对象也加密；导出用的对象字节是解密后的。
        let more = locked
            .write_file(&b"more"[..], std::path::Path::new("y"))
            .unwrap();
        assert!(is_sealed(
            &std::fs::read(locked.object_path(Kind::Blob, &more.blob)).unwrap()
        ));
        assert!(!is_sealed(
            &locked.read_object_file(Kind::Blob, &more.blob).unwrap()
        ));

        let r2 = change_password(&locked, &lock, "pw", "pw2").unwrap();
        assert!(r2.is_unlocked());
        assert!(crate::Repo::open(d.path()).unwrap().unlock("pw2").is_ok());

        let plain = decrypt_repo(&r2, &lock).unwrap();
        assert!(!plain.is_encrypted());
        assert!(!is_sealed(
            &std::fs::read(plain.object_path(Kind::Snapshot, &snap)).unwrap()
        ));
        assert_eq!(plain.reflog("g").unwrap().len(), 1);
        assert!(crate::fsck::fsck(&plain, true).unwrap().ok());
        drop(r);
    }
}
