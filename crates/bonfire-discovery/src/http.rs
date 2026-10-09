//! 联网请求的抽象：生产环境用 ureq，测试用 [`FakeHttp`]。所有请求只发送游戏 ID 或名称。

use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub trait Http: Send + Sync {
    /// 发送 GET 请求。网络错误返回 Err；HTTP 错误状态码放在 `status` 中返回。
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String>;

    /// 发送 POST 请求（正文为文本）。
    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        let _ = (url, headers, body);
        Err("不支持 POST".into())
    }
}

/// 离线：所有请求都失败（设置中关闭联网或 BONFIRE_OFFLINE=1 时使用）。
pub struct Offline;

impl Http for Offline {
    fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResponse, String> {
        Err(format!("已关闭联网：{url}"))
    }
}

/// 按主机限速并在 429 / 5xx 时退避重试（文档 4.3：请求走全局队列，按来源限速并退避重试）。
pub struct Throttled<H: Http> {
    inner: H,
    last: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// 测试中设为 0 以免等待。
    pub scale: f64,
}

impl<H: Http> Throttled<H> {
    pub fn new(inner: H) -> Self {
        Self {
            inner,
            last: Default::default(),
            scale: 1.0,
        }
    }

    fn host(url: &str) -> String {
        url.split("://")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string()
    }

    fn interval(host: &str) -> Duration {
        if host.ends_with("store.steampowered.com") {
            Duration::from_millis(1500)
        } else {
            Duration::from_millis(250)
        }
    }

    fn wait_turn(&self, url: &str) {
        let host = Self::host(url);
        let gap = Self::interval(&host).mul_f64(self.scale);
        let mut last = self.last.lock().unwrap();
        if let Some(t) = last.get(&host) {
            let elapsed = t.elapsed();
            if elapsed < gap {
                std::thread::sleep(gap - elapsed);
            }
        }
        last.insert(host, std::time::Instant::now());
    }

    fn retry(
        &self,
        url: &str,
        f: impl Fn() -> Result<HttpResponse, String>,
    ) -> Result<HttpResponse, String> {
        let mut delay = Duration::from_secs(2).mul_f64(self.scale);
        for attempt in 0..3 {
            self.wait_turn(url);
            let r = f()?;
            if (r.status == 429 || r.status >= 500) && attempt < 2 {
                tracing::debug!(url, status = r.status, "请求受限，退避重试");
                std::thread::sleep(delay);
                delay *= 2;
                continue;
            }
            return Ok(r);
        }
        unreachable!()
    }
}

impl<H: Http> Http for Throttled<H> {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
        self.retry(url, || self.inner.get(url, headers))
    }

    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        self.retry(url, || self.inner.post(url, headers, body))
    }
}

/// 基于 ureq 的实现（同步、rustls）。
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl Default for UreqHttp {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(concat!("Bonfire/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.send(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// 测试用：按 URL 前缀返回预设响应，并记录请求。
/// 记录的请求：(URL, 请求头；POST 正文以 `body` 头记录)。
pub type RecordedRequest = (String, Vec<(String, String)>);

#[derive(Default)]
pub struct FakeHttp {
    pub routes: Vec<(String, HttpResponse)>,
    pub requests: std::sync::Mutex<Vec<RecordedRequest>>,
}

impl FakeHttp {
    pub fn route(mut self, prefix: &str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.routes.push((
            prefix.to_string(),
            HttpResponse {
                status,
                headers: Vec::new(),
                body: body.into(),
            },
        ));
        self
    }

    pub fn route_with_headers(
        mut self,
        prefix: &str,
        status: u16,
        headers: &[(&str, &str)],
        body: impl Into<Vec<u8>>,
    ) -> Self {
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.routes.push((
            prefix.to_string(),
            HttpResponse {
                status,
                headers,
                body: body.into(),
            },
        ));
        self
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
        self.requests.lock().unwrap().push((
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self.routes
            .iter()
            .find(|(p, _)| url.starts_with(p.as_str()))
            .map(|(_, r)| r.clone())
            .ok_or_else(|| format!("离线：{url}"))
    }

    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        let mut h: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        h.push(("body".into(), body.to_string()));
        self.requests.lock().unwrap().push((url.to_string(), h));
        self.routes
            .iter()
            .find(|(p, _)| url.starts_with(p.as_str()))
            .map(|(_, r)| r.clone())
            .ok_or_else(|| format!("离线：{url}"))
    }
}

/// 测试用：按顺序返回预设响应。
pub struct SequenceHttp(pub std::sync::Mutex<Vec<HttpResponse>>);

impl Http for SequenceHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, String> {
        let mut v = self.0.lock().unwrap();
        if v.is_empty() {
            Err("没有更多响应".into())
        } else {
            Ok(v.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttled_retries_on_429() {
        let r = |s| HttpResponse {
            status: s,
            ..Default::default()
        };
        let mut t = Throttled::new(SequenceHttp(std::sync::Mutex::new(vec![
            r(429),
            r(503),
            r(200),
        ])));
        t.scale = 0.0;
        assert_eq!(t.get("https://x.test/a", &[]).unwrap().status, 200);
        let mut t = Throttled::new(SequenceHttp(std::sync::Mutex::new(vec![
            r(429),
            r(429),
            r(429),
        ])));
        t.scale = 0.0;
        assert_eq!(
            t.get("https://x.test/a", &[]).unwrap().status,
            429,
            "重试 2 次后返回最后的结果"
        );
    }

    #[test]
    fn host_parsing() {
        assert_eq!(
            Throttled::<Offline>::host("https://store.steampowered.com/api/appdetails?x"),
            "store.steampowered.com"
        );
        assert!(Offline.get("https://a", &[]).is_err());
    }
}
