//! 多语言文案。CLI 与 GUI 共用 `locales/*.json`（前端直接导入同一份文件）。
//!
//! 缺少翻译时回退到中文，中文也没有时返回键名。

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    ZhCn,
    En,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::ZhCn, Locale::En];

    pub fn code(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::En => "en",
        }
    }

    /// 接受 `zh-CN`、`zh`、`zh_CN`、`en`、`en-US` 等写法；其他语言返回 `None`。
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.trim().to_ascii_lowercase();
        let primary = lower.split(['-', '_']).next().unwrap_or("");
        match primary {
            "zh" => Some(Locale::ZhCn),
            "en" => Some(Locale::En),
            _ => None,
        }
    }

    /// 配置中的语言优先；否则跟随系统语言；都无法匹配时用中文。
    pub fn resolve(configured: Option<&str>) -> Self {
        configured
            .and_then(Self::parse)
            .or_else(|| sys_locale::get_locale().as_deref().and_then(Self::parse))
            .unwrap_or(Locale::ZhCn)
    }

    fn source(self) -> &'static str {
        match self {
            Locale::ZhCn => include_str!("../locales/zh-CN.json"),
            Locale::En => include_str!("../locales/en.json"),
        }
    }

    fn table(self) -> &'static HashMap<String, String> {
        static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
        static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
        let cell = match self {
            Locale::ZhCn => &ZH,
            Locale::En => &EN,
        };
        cell.get_or_init(|| serde_json::from_str(self.source()).expect("内置文案文件格式错误"))
    }
}

/// 取文案。
pub fn t(locale: Locale, key: &str) -> &str {
    if let Some(v) = locale.table().get(key) {
        return v;
    }
    if locale != Locale::ZhCn {
        tracing::debug!(key, locale = locale.code(), "缺少翻译，回退到中文");
        if let Some(v) = Locale::ZhCn.table().get(key) {
            return v;
        }
    }
    key
}

/// 取文案并替换 `{名称}` 占位符。
pub fn tf(locale: Locale, key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = t(locale, key).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        assert_eq!(Locale::parse("zh-CN"), Some(Locale::ZhCn));
        assert_eq!(Locale::parse("zh_TW"), Some(Locale::ZhCn));
        assert_eq!(Locale::parse("EN-us"), Some(Locale::En));
        assert_eq!(Locale::parse("pt-BR"), None);
        assert_eq!(Locale::parse(""), None);
    }

    #[test]
    fn configured_wins_and_unknown_falls_back() {
        assert_eq!(Locale::resolve(Some("en")), Locale::En);
        assert_eq!(Locale::resolve(Some("zh-CN")), Locale::ZhCn);
    }

    #[test]
    fn lookup_and_fallback() {
        assert_eq!(t(Locale::ZhCn, "app.name"), "篝火");
        assert_eq!(t(Locale::En, "app.name"), "Bonfire");
        assert_eq!(t(Locale::En, "no.such.key"), "no.such.key");
    }

    #[test]
    fn placeholders() {
        assert_eq!(tf(Locale::ZhCn, "backup.auto.added", &[("n", &3)]), "新增 3 个文件");
        assert_eq!(tf(Locale::En, "backup.auto.added", &[("n", &3)]), "added 3 files");
    }

    #[test]
    fn all_locales_have_same_keys() {
        let zh: Vec<_> = { let mut k: Vec<_> = Locale::ZhCn.table().keys().collect(); k.sort(); k };
        for l in Locale::ALL {
            let mut k: Vec<_> = l.table().keys().collect();
            k.sort();
            assert_eq!(k, zh, "{} 的文案键与中文不一致", l.code());
        }
    }
}
