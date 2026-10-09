//! 小工具：本地时间、机器名、文件大小格式化。

/// 本地时间，RFC 3339 带时区偏移，如 `2026-10-09T21:14:03+08:00`（文档 7.1）。
pub fn now_local_rfc3339() -> String {
    jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

/// 用于自动生成分支名的紧凑时间，如 `20261009-211403`。
pub fn now_compact() -> String {
    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string()
}

pub fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

/// 1024 进制的人类可读大小，如 `3.1 MB`。
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(3_250_586), "3.1 MB");
    }

    #[test]
    fn local_time_has_offset() {
        let s = now_local_rfc3339();
        assert_eq!(s.len(), 25, "{s}");
        assert!(s.contains('T'));
    }
}
