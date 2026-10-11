//! WebDAV 目标：坚果云、Nextcloud、群晖、AList 等。地址填到要放备份的文件夹，如
//! `https://dav.jianguoyun.com/dav/篝火/`；需要的上级文件夹自动创建。

use std::collections::HashSet;
use std::time::Duration;

use ureq::http;

use super::Remote;
use crate::error::{AppError, Result};

pub struct WebDav {
    agent: ureq::Agent,
    /// 以 `/` 结尾、各段已编码的地址。
    base: String,
    auth: Option<String>,
    made: HashSet<String>,
}

/// 地址中路径各段百分号编码（保留 `/`）。
fn encode(seg: &str) -> String {
    let mut out = String::new();
    for b in seg.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 把用户填写的地址规范为“协议 + 主机 + 编码后的路径 + /”。
fn normalize(url: &str) -> Result<String> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| AppError::Invalid("WebDAV 地址应以 https:// 或 http:// 开头".into()))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let segs: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| encode(&decode(s)))
        .collect();
    let mut out = format!("{scheme}://{host}/");
    for s in segs {
        out.push_str(&s);
        out.push('/');
    }
    Ok(out)
}

impl WebDav {
    pub fn new(url: &str, user: &str, password: Option<&str>) -> Result<Self> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            .user_agent(concat!("Bonfire/", env!("CARGO_PKG_VERSION")))
            .build();
        let auth = (!user.is_empty() || password.is_some()).then(|| {
            format!(
                "Basic {}",
                base64(format!("{user}:{}", password.unwrap_or("")).as_bytes())
            )
        });
        Ok(Self {
            agent: config.into(),
            base: normalize(url)?,
            auth,
            made: HashSet::new(),
        })
    }

    fn url(&self, rel: &str) -> String {
        let p: Vec<String> = rel
            .split('/')
            .filter(|s| !s.is_empty())
            .map(encode)
            .collect();
        format!("{}{}", self.base, p.join("/"))
    }

    fn send(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        depth: Option<&str>,
    ) -> Result<(u16, Vec<u8>)> {
        let mut b = http::Request::builder().method(method).uri(url);
        if let Some(a) = &self.auth {
            b = b.header("Authorization", a);
        }
        if let Some(d) = depth {
            b = b
                .header("Depth", d)
                .header("Content-Type", "application/xml; charset=utf-8");
        }
        let req = b
            .body(body.to_vec())
            .map_err(|e| AppError::Network(e.to_string()))?;
        let mut resp = self
            .agent
            .run(req)
            .map_err(|e| AppError::Network(format!("WebDAV {method} {url}：{e}")))?;
        let status = resp.status().as_u16();
        let data = resp
            .body_mut()
            .with_config()
            .limit(1024 * 1024 * 1024)
            .read_to_vec()
            .unwrap_or_default();
        if status == 401 || status == 403 {
            return Err(AppError::Network(format!(
                "WebDAV 拒绝访问（{status}）：请检查用户名与密码（坚果云等需使用“应用密码”）"
            )));
        }
        Ok((status, data))
    }

    fn mkcol(&mut self, dir_url: &str) -> Result<()> {
        if self.made.contains(dir_url) {
            return Ok(());
        }
        let (st, _) = self.send("MKCOL", dir_url, b"", None)?;
        // 201 新建；405 已存在。
        if !(st == 201 || st == 405 || st == 200 || st == 301) {
            return Err(AppError::Network(format!(
                "WebDAV 创建文件夹失败（{st}）：{dir_url}"
            )));
        }
        self.made.insert(dir_url.to_string());
        Ok(())
    }

    /// 逐级创建 `rel` 的上级文件夹（含根文件夹）。
    fn ensure_parents(&mut self, rel: &str) -> Result<()> {
        let base = self.base.clone();
        self.ensure_base()?;
        let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        let mut url = base;
        for s in &segs[..segs.len().saturating_sub(1)] {
            url.push_str(&encode(s));
            url.push('/');
            self.mkcol(&url)?;
        }
        Ok(())
    }

    /// 根文件夹（地址本身）不存在时逐级创建。
    fn ensure_base(&mut self) -> Result<()> {
        if self.made.contains(&self.base) {
            return Ok(());
        }
        let (st, _) = self.send("PROPFIND", &self.base.clone(), PROPFIND0, Some("0"))?;
        if st == 404 {
            let base = self.base.clone();
            let (scheme_host, path) = base.split_at(base.find("://").map(|i| i + 3).unwrap_or(0));
            let (host, path) = path.split_once('/').unwrap_or((path, ""));
            let mut url = format!("{scheme_host}{host}/");
            for s in path.split('/').filter(|s| !s.is_empty()) {
                url.push_str(s);
                url.push('/');
                self.mkcol(&url)?;
            }
        } else if !(200..300).contains(&st) {
            return Err(AppError::Network(format!(
                "WebDAV 地址无法访问（{st}）：{}",
                self.base
            )));
        }
        self.made.insert(self.base.clone());
        Ok(())
    }
}

const PROPFIND0: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"#;

/// 从 PROPFIND 结果中取出各项：(地址路径, 是否文件夹, 大小)。容忍不同的命名空间前缀。
fn parse_multistatus(xml: &str) -> Vec<(String, bool, u64)> {
    let mut out = Vec::new();
    for block in xml
        .split(":response>")
        .chain(xml.split("<response>").skip(1))
    {
        let Some(href) = tag(block, "href") else {
            continue;
        };
        let collection = block.contains("collection");
        let size = tag(block, "getcontentlength")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        out.push((href.trim().to_string(), collection, size));
    }
    out.sort();
    out.dedup();
    out
}

/// `<x:name>文本</x:name>` 或 `<name>文本</name>` 中的文本。
fn tag(block: &str, name: &str) -> Option<String> {
    for open in [format!(":{name}>"), format!("<{name}>")] {
        if let Some(i) = block.find(&open) {
            let rest = &block[i + open.len()..];
            let end = rest.find("</")?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

impl Remote for WebDav {
    fn test(&mut self) -> Result<String> {
        self.ensure_base()?;
        let probe = format!("{}.bonfire-write-test", self.base);
        let (st, _) = self.send("PUT", &probe, b"ok", None)?;
        if !(200..300).contains(&st) {
            return Err(AppError::Network(format!("WebDAV 无法写入（{st}）")));
        }
        let _ = self.send("DELETE", &probe, b"", None);
        Ok(format!("可以写入 {}", decode(&self.base)))
    }

    fn put(&mut self, rel: &str, data: &[u8]) -> Result<()> {
        self.ensure_parents(rel)?;
        let url = self.url(rel);
        let (st, _) = self.send("PUT", &url, data, None)?;
        if !(200..300).contains(&st) {
            return Err(AppError::Network(format!("WebDAV 上传失败（{st}）：{rel}")));
        }
        Ok(())
    }

    fn delete(&mut self, rel: &str) -> Result<()> {
        let (st, _) = self.send("DELETE", &self.url(rel), b"", None)?;
        if !(200..300).contains(&st) && st != 404 {
            return Err(AppError::Network(format!("WebDAV 删除失败（{st}）：{rel}")));
        }
        Ok(())
    }

    /// 逐层列出（不少服务关闭了 Depth: infinity）。
    fn list(&mut self) -> Result<Vec<(String, u64)>> {
        let base_path = {
            let after = self.base.find("://").map(|i| i + 3).unwrap_or(0);
            let p = &self.base[after..];
            p.find('/')
                .map(|i| p[i..].to_string())
                .unwrap_or_else(|| "/".into())
        };
        let mut out = Vec::new();
        let mut queue = vec![self.base.clone()];
        while let Some(dir) = queue.pop() {
            let (st, body) = self.send("PROPFIND", &dir, PROPFIND0, Some("1"))?;
            if st == 404 {
                continue;
            }
            if st != 207 {
                return Err(AppError::Network(format!("WebDAV 列出文件失败（{st}）")));
            }
            let dir_path = {
                let after = dir.find("://").map(|i| i + 3).unwrap_or(0);
                let p = &dir[after..];
                decode(&p[p.find('/').unwrap_or(0)..])
            };
            for (href, is_dir, size) in parse_multistatus(&String::from_utf8_lossy(&body)) {
                // href 可能是完整地址或路径。
                let path = match href.find("://") {
                    Some(i) => {
                        let p = &href[i + 3..];
                        decode(&p[p.find('/').unwrap_or(0)..])
                    }
                    None => decode(&href),
                };
                if path.trim_end_matches('/') == dir_path.trim_end_matches('/') {
                    continue;
                }
                let base_dec = decode(&base_path);
                let Some(rel) = path.strip_prefix(&base_dec) else {
                    continue;
                };
                let rel = rel.trim_matches('/').to_string();
                if rel.is_empty() {
                    continue;
                }
                if is_dir {
                    queue.push(self.url(&rel) + "/");
                } else if !rel.ends_with(".bonfire-write-test") {
                    out.push((rel, size));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn get(&mut self, rel: &str) -> Result<Vec<u8>> {
        let (st, body) = self.send("GET", &self.url(rel), b"", None)?;
        if st != 200 {
            return Err(AppError::Network(format!("WebDAV 下载失败（{st}）：{rel}")));
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(
            normalize("https://dav.example.com/dav/篝火 备份").unwrap(),
            "https://dav.example.com/dav/%E7%AF%9D%E7%81%AB%20%E5%A4%87%E4%BB%BD/"
        );
        assert_eq!(decode("%E7%AF%9D%E7%81%AB"), "篝火");
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/dav/a/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/dav/a/b.txt</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>12</d:getcontentlength></d:prop></d:propstat></d:response></d:multistatus>"#;
        let v = parse_multistatus(xml);
        assert!(v.contains(&("/dav/a/".into(), true, 0)));
        assert!(v.contains(&("/dav/a/b.txt".into(), false, 12)));
    }
}
