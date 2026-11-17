use crate::Result;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(()),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

pub fn parse_json(input: &str) -> Result<Json> {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let value = p.value()?;
    p.ws();
    if p.pos != p.chars.len() {
        return Err("trailing JSON input".into());
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn value(&mut self) -> Result<Json> {
        self.ws();
        match self.peek() {
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(())),
            Some('f') => self.literal("false", Json::Bool(())),
            Some('"') => self.string().map(Json::String),
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some('-' | '0'..='9') => self.number(),
            _ => Err("invalid JSON value".into()),
        }
    }

    fn literal(&mut self, lit: &str, value: Json) -> Result<Json> {
        for expected in lit.chars() {
            if self.bump() != Some(expected) {
                return Err(format!("expected {lit}"));
            }
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String> {
        self.expect('"')?;
        let mut out = String::new();
        while let Some(ch) = self.bump() {
            match ch {
                '"' => return Ok(out),
                '\\' => match self
                    .bump()
                    .ok_or_else(|| "unterminated escape".to_string())?
                {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            code = code * 16
                                + self
                                    .bump()
                                    .and_then(|c| c.to_digit(16))
                                    .ok_or_else(|| "invalid unicode escape".to_string())?;
                        }
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                    }
                    other => return Err(format!("invalid escape {other}")),
                },
                other => out.push(other),
            }
        }
        Err("unterminated string".into())
    }

    fn array(&mut self) -> Result<Json> {
        self.expect('[')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.peek() == Some(']') {
                self.bump();
                return Ok(Json::Array(out));
            }
            out.push(self.value()?);
            self.ws();
            match self.bump() {
                Some(',') => {}
                Some(']') => return Ok(Json::Array(out)),
                _ => return Err("expected ',' or ']'".into()),
            }
        }
    }

    fn object(&mut self) -> Result<Json> {
        self.expect('{')?;
        let mut out = BTreeMap::new();
        loop {
            self.ws();
            if self.peek() == Some('}') {
                self.bump();
                return Ok(Json::Object(out));
            }
            let key = self.string()?;
            self.ws();
            self.expect(':')?;
            out.insert(key, self.value()?);
            self.ws();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Json::Object(out)),
                _ => return Err("expected ',' or '}'".into()),
            }
        }
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.pos;
        while matches!(self.peek(), Some('-' | '+' | '.' | 'e' | 'E' | '0'..='9')) {
            self.bump();
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        Ok(Json::Number(
            raw.parse().map_err(|_| format!("invalid number {raw}"))?,
        ))
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.bump();
        }
    }

    fn expect(&mut self, ch: char) -> Result<()> {
        (self.bump() == Some(ch))
            .then_some(())
            .ok_or_else(|| format!("expected {ch}"))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn get_path(&self, path: &[&str]) -> Option<&Json> {
        let mut cur = self;
        for key in path {
            cur = cur.get(key)?;
        }
        Some(cur)
    }

    pub fn string(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(v) => Some(v),
            _ => None,
        }
    }
}

pub fn required_string<'a>(json: &'a Json, path: &[&str]) -> Result<&'a str> {
    let value = string_field(json, path)?;
    if value.is_empty() {
        return Err(format!("empty string field {}", path.join(".")));
    }
    Ok(value)
}

pub fn string_field<'a>(json: &'a Json, path: &[&str]) -> Result<&'a str> {
    json.get_path(path)
        .and_then(Json::string)
        .ok_or_else(|| format!("missing string field {}", path.join(".")))
}

pub fn json_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}

pub fn unescape_json_string(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\\\", "\\")
}
