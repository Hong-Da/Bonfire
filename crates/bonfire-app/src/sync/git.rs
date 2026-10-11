//! GitHub / Gitee 目标：内置 libgit2，用访问令牌通过 HTTPS 推送，不需要安装 Git。
//!
//! - 本机缓存目录中有一个裸仓库（`cache/sync/<目标>.git`），每批改动在上一次的提交上更新目录树后提交并推送；
//!   第一次同步按批（约 200 MB）分几次推送，避免一次推送过大被拒绝。
//! - 令牌：GitHub 用户名任意（用 `x-access-token`）；Gitee 的用户名必须是登录名（个人空间地址中的名称），
//!   填错会被拒绝（403），所以依次尝试：用令牌查到的登录名、填写的用户名、仓库地址中的所有者。
//! - 公开仓库不同步（存档、截图会被所有人看到）：推送前用 GitHub / Gitee 的接口检查仓库是否私有。
//! - 压缩远程仓库：用一个只含当前内容、没有父提交的新提交强制替换远程分支，去掉已删除内容的旧版本。

use std::path::{Path, PathBuf};
use std::time::Duration;

use git2::build::TreeUpdateBuilder;
use git2::{
    Cred, FetchOptions, FileMode, Oid, PushOptions, RemoteCallbacks, Repository, Signature,
};

use super::Remote;
use super::target::SyncKind;
use crate::error::{AppError, Result};

pub struct GitRemote {
    repo: Repository,
    kind: SyncKind,
    url: String,
    branch: String,
    user: String,
    token: Option<String>,
    /// 当前分支的提交（远程最新或本次已提交的）。
    base: Option<Oid>,
    updates: Vec<(String, Option<Oid>)>,
}

fn gerr(e: git2::Error) -> AppError {
    let msg = e.message().to_string();
    let hint = if is_auth_error(&msg) {
        "（被拒绝访问：请检查令牌是否正确、是否有读写这个仓库的权限；Gitee 的用户名要填登录名，即个人空间地址 gitee.com/<名称> 中的名称，可以留空自动获取）"
    } else if msg.contains("404") || msg.to_lowercase().contains("not found") {
        "（请检查仓库地址，仓库需要先在网站上创建）"
    } else {
        ""
    };
    AppError::Network(format!("Git：{msg}{hint}"))
}

fn is_auth_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("401") || m.contains("403") || m.contains("auth")
}

/// 用 Gitee 令牌查登录名（个人空间地址中的名称）。
pub fn gitee_login(token: &str) -> Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(concat!("Bonfire/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = agent
        .get(format!(
            "https://gitee.com/api/v5/user?access_token={token}"
        ))
        .call()
        .map_err(|e| AppError::Network(e.to_string()))?;
    if resp.status().as_u16() != 200 {
        return Err(AppError::Network(format!(
            "查询 Gitee 账号失败（{}）",
            resp.status().as_u16()
        )));
    }
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AppError::Network(e.to_string()))?;
    v.get("login")
        .and_then(|l| l.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Network("Gitee 没有返回登录名".into()))
}

/// Gitee 推送时依次尝试的用户名（去重、去空）。
fn gitee_users(url: &str, user: &str, token: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |u: String| {
        let u = u.trim().to_string();
        if !u.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&u)) {
            out.push(u);
        }
    };
    if let Some(t) = token.filter(|_| !is_local(url)) {
        match gitee_login(t) {
            Ok(l) => push(l),
            Err(e) => tracing::warn!("查询 Gitee 登录名失败：{e}"),
        }
    }
    push(user.to_string());
    if let Some((owner, _)) = owner_repo(url) {
        push(owner);
    }
    out
}

fn is_local(url: &str) -> bool {
    url.starts_with("file://") || Path::new(url).is_absolute()
}

/// `https://gitee.com/用户/仓库.git` → (用户, 仓库)。
pub fn owner_repo(url: &str) -> Option<(String, String)> {
    let rest = url.trim().split("://").nth(1)?;
    let mut parts = rest.split('/').skip(1).filter(|s| !s.is_empty());
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    Some((owner, repo))
}

impl GitRemote {
    pub fn open(
        cache: &Path,
        kind: SyncKind,
        url: &str,
        branch: &str,
        user: &str,
        token: Option<String>,
    ) -> Result<Self> {
        let repo = match Repository::open_bare(cache) {
            Ok(r) => r,
            Err(_) => {
                std::fs::create_dir_all(cache).map_err(|e| AppError::io(cache, e))?;
                Repository::init_bare(cache).map_err(gerr)?
            }
        };
        let branch = if branch.trim().is_empty() {
            "main".to_string()
        } else {
            branch.trim().to_string()
        };
        let mut me = Self {
            repo,
            kind,
            url: url.trim().to_string(),
            branch,
            user: user.to_string(),
            token,
            base: None,
            updates: Vec::new(),
        };
        if me.kind == SyncKind::Gitee {
            let users = gitee_users(&me.url, user, me.token.as_deref());
            let mut last = None;
            for u in users {
                me.user = u;
                match me.fetch() {
                    Ok(()) => return Ok(me),
                    Err(AppError::Network(m)) if is_auth_error(&m) => {
                        last = Some(AppError::Network(m))
                    }
                    Err(e) => return Err(e),
                }
            }
            if let Some(e) = last {
                return Err(e);
            }
            me.user = user.to_string();
        }
        me.fetch()?;
        Ok(me)
    }

    fn callbacks(&self) -> RemoteCallbacks<'_> {
        let mut cb = RemoteCallbacks::new();
        let user = if self.kind == SyncKind::Github || self.user.is_empty() {
            "x-access-token".to_string()
        } else {
            self.user.clone()
        };
        let token = self.token.clone();
        cb.credentials(move |_url, _from_url, _allowed| match &token {
            Some(t) => Cred::userpass_plaintext(&user, t),
            None => Err(git2::Error::from_str("没有设置访问令牌")),
        });
        cb
    }

    fn remote(&self) -> Result<git2::Remote<'_>> {
        let url = if self.url.starts_with("file://") || !is_local(&self.url) {
            self.url.clone()
        } else {
            self.url.replace('\\', "/")
        };
        self.repo.remote_anonymous(&url).map_err(gerr)
    }

    /// 取回远程分支（远程是空仓库时没有分支）。
    fn fetch(&mut self) -> Result<()> {
        {
            let mut remote = self.remote()?;
            let mut opts = FetchOptions::new();
            opts.remote_callbacks(self.callbacks());
            let spec = format!("+refs/heads/{0}:refs/remotes/origin/{0}", self.branch);
            match remote.fetch(&[spec.as_str()], Some(&mut opts), None) {
                Ok(()) => {}
                // 空仓库没有这个分支：不是错误。
                Err(e)
                    if e.message().contains("couldn't find remote ref")
                        || e.code() == git2::ErrorCode::NotFound => {}
                Err(e) => return Err(gerr(e)),
            }
        }
        self.base = self
            .repo
            .refname_to_id(&format!("refs/remotes/origin/{}", self.branch))
            .ok();
        Ok(())
    }

    fn base_tree(&self) -> Result<Option<git2::Tree<'_>>> {
        Ok(match self.base {
            Some(c) => Some(
                self.repo
                    .find_commit(c)
                    .map_err(gerr)?
                    .tree()
                    .map_err(gerr)?,
            ),
            None => None,
        })
    }

    fn push(&self, force: bool) -> Result<()> {
        let mut remote = self.remote()?;
        let mut opts = PushOptions::new();
        let mut cb = self.callbacks();
        let rejected = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let r2 = rejected.clone();
        cb.push_update_reference(move |refname, status| {
            if let Some(s) = status {
                *r2.borrow_mut() = Some(format!("{refname}：{s}"));
            }
            Ok(())
        });
        opts.remote_callbacks(cb);
        let spec = format!(
            "{}refs/heads/{1}:refs/heads/{1}",
            if force { "+" } else { "" },
            self.branch
        );
        remote
            .push(&[spec.as_str()], Some(&mut opts))
            .map_err(gerr)?;
        if let Some(why) = rejected.borrow().clone() {
            return Err(AppError::Network(format!("远程拒绝了推送：{why}")));
        }
        Ok(())
    }

    fn signature() -> Signature<'static> {
        Signature::now("Bonfire", "bonfire@localhost").expect("签名有效")
    }

    /// 压缩远程：一个只含当前内容、没有父提交的新提交强制替换远程分支。
    pub fn compact(&mut self) -> Result<()> {
        let c = {
            let Some(tree) = self.base_tree()? else {
                return Ok(());
            };
            self.repo
                .commit(
                    None,
                    &Self::signature(),
                    &Self::signature(),
                    "篝火：压缩远程仓库（只保留当前内容）",
                    &tree,
                    &[],
                )
                .map_err(gerr)?
        };
        self.repo
            .reference(&format!("refs/heads/{}", self.branch), c, true, "compact")
            .map_err(gerr)?;
        self.push(true)?;
        self.base = Some(c);
        Ok(())
    }
}

impl Remote for GitRemote {
    fn test(&mut self) -> Result<String> {
        let mut remote = self.remote()?;
        remote
            .connect_auth(git2::Direction::Push, Some(self.callbacks()), None)
            .map_err(gerr)?;
        let _ = remote.disconnect();
        Ok(match self.base {
            Some(_) => format!("可以连接，分支 {} 已有内容", self.branch),
            None => format!("可以连接，分支 {} 还没有内容", self.branch),
        })
    }

    fn put(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let oid = self.repo.blob(data).map_err(gerr)?;
        self.updates.push((path.to_string(), Some(oid)));
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<()> {
        self.updates.push((path.to_string(), None));
        Ok(())
    }

    fn commit(&mut self, message: &str) -> Result<()> {
        if self.updates.is_empty() {
            return Ok(());
        }
        let c = self.commit_tree(message)?;
        self.repo
            .reference(&format!("refs/heads/{}", self.branch), c, true, "sync")
            .map_err(gerr)?;
        self.push(false)?;
        self.base = Some(c);
        self.updates.clear();
        Ok(())
    }

    fn batch_bytes(&self) -> u64 {
        200 * 1024 * 1024
    }

    fn list(&mut self) -> Result<Vec<(String, u64)>> {
        self.list_files()
    }

    fn get(&mut self, path: &str) -> Result<Vec<u8>> {
        let t = self
            .base_tree()?
            .ok_or_else(|| AppError::Invalid("远程分支没有内容".into()))?;
        let e = t.get_path(Path::new(path)).map_err(gerr)?;
        Ok(self
            .repo
            .find_blob(e.id())
            .map_err(gerr)?
            .content()
            .to_vec())
    }
}

impl GitRemote {
    /// 在当前提交上应用改动，生成新提交（还没推送）。
    fn commit_tree(&self, message: &str) -> Result<Oid> {
        let mut b = TreeUpdateBuilder::new();
        for (p, oid) in &self.updates {
            match oid {
                Some(o) => b.upsert(p.as_str(), *o, FileMode::Blob),
                None => b.remove(p.as_str()),
            };
        }
        let tree_id = match self.base_tree()? {
            Some(t) => b.create_updated(&self.repo, &t).map_err(gerr)?,
            None => {
                let empty = self
                    .repo
                    .treebuilder(None)
                    .map_err(gerr)?
                    .write()
                    .map_err(gerr)?;
                let t = self.repo.find_tree(empty).map_err(gerr)?;
                b.create_updated(&self.repo, &t).map_err(gerr)?
            }
        };
        let tree = self.repo.find_tree(tree_id).map_err(gerr)?;
        let parents: Vec<git2::Commit> = match self.base {
            Some(c) => vec![self.repo.find_commit(c).map_err(gerr)?],
            None => vec![],
        };
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        self.repo
            .commit(
                None,
                &Self::signature(),
                &Self::signature(),
                message,
                &tree,
                &refs,
            )
            .map_err(gerr)
    }

    fn list_files(&self) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        if let Some(t) = self.base_tree()? {
            t.walk(git2::TreeWalkMode::PreOrder, |dir, e| {
                if e.kind() == Some(git2::ObjectType::Blob) {
                    let size = self
                        .repo
                        .find_blob(e.id())
                        .map(|b| b.size() as u64)
                        .unwrap_or(0);
                    out.push((format!("{dir}{}", e.name().unwrap_or("")), size));
                }
                git2::TreeWalkResult::Ok
            })
            .map_err(gerr)?;
        }
        Ok(out)
    }
}

/// 检查 GitHub / Gitee 上的仓库是否私有：Ok(true) 私有，Ok(false) 公开。
pub fn check_private(kind: SyncKind, url: &str, token: &str) -> Result<bool> {
    let (owner, repo) =
        owner_repo(url).ok_or_else(|| AppError::Invalid(format!("仓库地址不对：{url}")))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(concat!("Bonfire/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = match kind {
        SyncKind::Github => agent
            .get(format!("https://api.github.com/repos/{owner}/{repo}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .call(),
        _ => agent
            .get(format!(
                "https://gitee.com/api/v5/repos/{owner}/{repo}?access_token={token}"
            ))
            .call(),
    }
    .map_err(|e| AppError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 401 {
        return Err(AppError::Network("访问令牌无效或已过期".into()));
    }
    if status == 404 {
        return Err(AppError::Network(
            "找不到仓库：请先在网站上创建，或检查令牌是否有访问这个仓库的权限".into(),
        ));
    }
    if status != 200 {
        return Err(AppError::Network(format!("查询仓库信息失败（{status}）")));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AppError::Network(e.to_string()))?;
    Ok(v.get("private").and_then(|p| p.as_bool()).unwrap_or(false))
}

/// 缓存中的裸仓库位置。
pub fn cache_dir(cache: &Path, id: &str) -> PathBuf {
    cache.join("sync").join(format!("{id}.git"))
}

/// 目标地址是否为本机路径（测试用的本地裸仓库）。
pub fn local(url: &str) -> bool {
    is_local(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitee_user_candidates() {
        // 没有令牌时不查接口：先填写的用户名，再仓库地址中的所有者；去重、去空。
        let u = gitee_users("https://gitee.com/Hong_Xiao_Feng/bak.git", "xiaofenghong", None);
        assert_eq!(u, ["xiaofenghong", "Hong_Xiao_Feng"]);
        let u = gitee_users("https://gitee.com/Hong_Xiao_Feng/bak.git", " ", None);
        assert_eq!(u, ["Hong_Xiao_Feng"]);
        let u = gitee_users("https://gitee.com/hong_xiao_feng/bak.git", "Hong_Xiao_Feng", None);
        assert_eq!(u, ["Hong_Xiao_Feng"]);
        assert!(is_auth_error("request failed with status code: 403"));
        assert!(!is_auth_error("couldn't find remote ref"));
    }
}
