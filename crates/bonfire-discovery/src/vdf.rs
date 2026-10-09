//! Valve KeyValues 文本格式（`.vdf` / `.acf`）的最小解析器。
//!
//! 只支持 Steam 本地文件用到的子集：带引号或不带引号的键值、嵌套块、`//` 注释、`\\` `\"` 转义。

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Obj(VdfObj),
}

/// 键不区分大小写（Steam 文件中大小写不统一，如 `LastPlayed` / `lastplayed`）；保留首次出现的值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VdfObj(BTreeMap<String, (String, Vdf)>);

impl VdfObj {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        self.0.get(&key.to_ascii_lowercase()).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn obj(&self, key: &str) -> Option<&VdfObj> {
        match self.get(key)? {
            Vdf::Obj(o) => Some(o),
            Vdf::Str(_) => None,
        }
    }

    /// 按路径取子对象，如 `["Software", "Valve", "Steam", "apps"]`。
    pub fn path(&self, keys: &[&str]) -> Option<&VdfObj> {
        keys.iter().try_fold(self, |o, k| o.obj(k))
    }

    /// 按原始大小写遍历。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Vdf)> {
        self.0.values().map(|(k, v)| (k.as_str(), v))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("VDF 第 {line} 行：{msg}")]
pub struct VdfError {
    pub line: usize,
    pub msg: &'static str,
}

/// 解析整个文件，返回顶层对象。
pub fn parse(text: &str) -> Result<VdfObj, VdfError> {
    let mut p = Parser {
        chars: text.trim_start_matches('\u{feff}').chars().collect(),
        pos: 0,
        line: 1,
    };
    let obj = p.object(false)?;
    Ok(obj)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

enum Token {
    Str(String),
    Open,
    Close,
    Eof,
}

impl Parser {
    fn err(&self, msg: &'static str) -> VdfError {
        VdfError {
            line: self.line,
            msg,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&c) = self.chars.get(self.pos) {
            if c == '\n' {
                self.line += 1;
                self.pos += 1;
            } else if c.is_whitespace() {
                self.pos += 1;
            } else if c == '/' && self.chars.get(self.pos + 1) == Some(&'/') {
                while self.chars.get(self.pos).is_some_and(|&c| c != '\n') {
                    self.pos += 1;
                }
            } else if c == '[' {
                // 条件标记如 [$WIN32]，忽略。
                while self
                    .chars
                    .get(self.pos)
                    .is_some_and(|&c| c != ']' && c != '\n')
                {
                    self.pos += 1;
                }
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn token(&mut self) -> Result<Token, VdfError> {
        self.skip_ws();
        let Some(&c) = self.chars.get(self.pos) else {
            return Ok(Token::Eof);
        };
        match c {
            '{' => {
                self.pos += 1;
                Ok(Token::Open)
            }
            '}' => {
                self.pos += 1;
                Ok(Token::Close)
            }
            '"' => {
                self.pos += 1;
                let mut s = String::new();
                loop {
                    let Some(&c) = self.chars.get(self.pos) else {
                        return Err(self.err("字符串未结束"));
                    };
                    self.pos += 1;
                    match c {
                        '"' => break,
                        '\\' => {
                            let n = self
                                .chars
                                .get(self.pos)
                                .copied()
                                .ok_or_else(|| self.err("转义未结束"))?;
                            self.pos += 1;
                            s.push(match n {
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                        }
                        '\n' => {
                            self.line += 1;
                            s.push(c);
                        }
                        _ => s.push(c),
                    }
                }
                Ok(Token::Str(s))
            }
            _ => {
                let start = self.pos;
                while self
                    .chars
                    .get(self.pos)
                    .is_some_and(|&c| !c.is_whitespace() && c != '{' && c != '}' && c != '"')
                {
                    self.pos += 1;
                }
                Ok(Token::Str(self.chars[start..self.pos].iter().collect()))
            }
        }
    }

    fn object(&mut self, nested: bool) -> Result<VdfObj, VdfError> {
        let mut obj = VdfObj::default();
        loop {
            let key = match self.token()? {
                Token::Str(k) => k,
                Token::Close if nested => return Ok(obj),
                Token::Eof if !nested => return Ok(obj),
                Token::Eof => return Err(self.err("缺少 }")),
                Token::Close => return Err(self.err("多余的 }")),
                Token::Open => return Err(self.err("缺少键名")),
            };
            let value = match self.token()? {
                Token::Str(v) => Vdf::Str(v),
                Token::Open => Vdf::Obj(self.object(true)?),
                _ => return Err(self.err("缺少值")),
            };
            obj.0
                .entry(key.to_ascii_lowercase())
                .or_insert((key, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"D:\\01Softwares\\Steam"
		"apps"
		{
			"228980"		"757576727"
			"1030300"		"8207380004"
		}
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"   // 注释
		"apps" { "70" "1" }
	}
}"#;

    #[test]
    fn parses_library_folders() {
        let v = parse(LIBRARY).unwrap();
        let lf = v.obj("libraryfolders").unwrap();
        assert_eq!(
            lf.obj("0").unwrap().str("path"),
            Some(r"D:\01Softwares\Steam")
        );
        let apps: Vec<_> = lf
            .obj("0")
            .unwrap()
            .obj("apps")
            .unwrap()
            .iter()
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(apps, ["1030300", "228980"]);
        assert_eq!(lf.path(&["1", "apps"]).unwrap().str("70"), Some("1"));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let v = parse("\"AppState\" { \"LastPlayed\" \"5\" \"name\" \"空洞骑士 ã\" }").unwrap();
        let a = v.obj("appstate").unwrap();
        assert_eq!(a.str("lastplayed"), Some("5"));
        assert_eq!(a.str("NAME"), Some("空洞骑士 ã"));
    }

    #[test]
    fn errors_have_line_numbers() {
        assert_eq!(parse("\"a\"\n{\n\"b\" \"c\"\n").unwrap_err().msg, "缺少 }");
        assert!(parse("\"a\" \"unterminated").is_err());
        assert!(parse("}").is_err());
    }

    #[test]
    fn unquoted_and_conditionals() {
        let v = parse("root { key value [$WIN32] other \"x\" }").unwrap();
        assert_eq!(v.obj("root").unwrap().str("key"), Some("value"));
        assert_eq!(v.obj("root").unwrap().str("other"), Some("x"));
    }
}
