use std::fmt;

// ── Core types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum JsonNumber {
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(JsonNumber),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

#[derive(Debug)]
pub struct JsonError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON error at position {}: {}", self.position, self.message)
    }
}

// ── Accessors ───────────────────────────────────────────────────────────────

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(pairs) => {
                pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    pub fn index(&self, i: usize) -> Option<&JsonValue> {
        match self {
            JsonValue::Array(items) => items.get(i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(JsonNumber::Int(n)) => Some(*n),
            JsonValue::Number(JsonNumber::Float(f)) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Number(JsonNumber::Float(f)) => Some(*f),
            JsonValue::Number(JsonNumber::Int(n)) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        match self {
            JsonValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(String, JsonValue)>> {
        match self {
            JsonValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsonValue::Null)
    }
}

// ── Serializer ──────────────────────────────────────────────────────────────

impl JsonValue {
    pub fn to_json_string(&self) -> String {
        let mut buf = String::new();
        serialize(self, &mut buf);
        buf
    }
}

fn serialize(val: &JsonValue, buf: &mut String) {
    match val {
        JsonValue::Null => buf.push_str("null"),
        JsonValue::Bool(true) => buf.push_str("true"),
        JsonValue::Bool(false) => buf.push_str("false"),
        JsonValue::Number(JsonNumber::Int(n)) => {
            buf.push_str(&n.to_string());
        }
        JsonValue::Number(JsonNumber::Float(f)) => {
            if f.is_infinite() || f.is_nan() {
                buf.push_str("null");
            } else {
                buf.push_str(&format!("{}", f));
            }
        }
        JsonValue::String(s) => {
            buf.push('"');
            escape_string(s, buf);
            buf.push('"');
        }
        JsonValue::Array(items) => {
            buf.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                serialize(item, buf);
            }
            buf.push(']');
        }
        JsonValue::Object(pairs) => {
            buf.push('{');
            for (i, (key, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push('"');
                escape_string(key, buf);
                buf.push_str("\":");
                serialize(val, buf);
            }
            buf.push('}');
        }
    }
}

fn escape_string(s: &str, buf: &mut String) {
    for ch in s.chars() {
        match ch {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                buf.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => buf.push(c),
        }
    }
}

// ── Builder pattern ─────────────────────────────────────────────────────────

pub struct ObjectBuilder {
    pairs: Vec<(String, JsonValue)>,
}

impl ObjectBuilder {
    pub fn field(mut self, key: &str, val: JsonValue) -> Self {
        self.pairs.push((key.to_string(), val));
        self
    }

    pub fn field_str(mut self, key: &str, val: &str) -> Self {
        self.pairs.push((key.to_string(), JsonValue::String(val.to_string())));
        self
    }

    pub fn field_i64(mut self, key: &str, val: i64) -> Self {
        self.pairs.push((key.to_string(), JsonValue::Number(JsonNumber::Int(val))));
        self
    }

    pub fn field_bool(mut self, key: &str, val: bool) -> Self {
        self.pairs.push((key.to_string(), JsonValue::Bool(val)));
        self
    }

    pub fn field_null(mut self, key: &str) -> Self {
        self.pairs.push((key.to_string(), JsonValue::Null));
        self
    }

    pub fn build(self) -> JsonValue {
        JsonValue::Object(self.pairs)
    }
}

pub fn json_obj() -> ObjectBuilder {
    ObjectBuilder { pairs: Vec::new() }
}

pub struct ArrayBuilder {
    items: Vec<JsonValue>,
}

impl ArrayBuilder {
    pub fn push(mut self, val: JsonValue) -> Self {
        self.items.push(val);
        self
    }

    pub fn push_str(mut self, val: &str) -> Self {
        self.items.push(JsonValue::String(val.to_string()));
        self
    }

    pub fn build(self) -> JsonValue {
        JsonValue::Array(self.items)
    }
}

pub fn json_arr() -> ArrayBuilder {
    ArrayBuilder { items: Vec::new() }
}

// ── Parser ──────────────────────────────────────────────────────────────────

pub fn parse(input: &str) -> Result<JsonValue, JsonError> {
    let mut pos = 0;
    let bytes = input.as_bytes();
    skip_whitespace(bytes, &mut pos);
    let val = parse_value(bytes, input, &mut pos)?;
    skip_whitespace(bytes, &mut pos);
    if pos != bytes.len() {
        return Err(JsonError {
            message: "unexpected trailing content".into(),
            position: pos,
        });
    }
    Ok(val)
}

fn parse_value(bytes: &[u8], input: &str, pos: &mut usize) -> Result<JsonValue, JsonError> {
    skip_whitespace(bytes, pos);
    if *pos >= bytes.len() {
        return Err(JsonError {
            message: "unexpected end of input".into(),
            position: *pos,
        });
    }
    match bytes[*pos] {
        b'"' => parse_string(bytes, input, pos).map(JsonValue::String),
        b'{' => parse_object(bytes, input, pos),
        b'[' => parse_array(bytes, input, pos),
        b't' => parse_literal(bytes, pos, b"true", JsonValue::Bool(true)),
        b'f' => parse_literal(bytes, pos, b"false", JsonValue::Bool(false)),
        b'n' => parse_literal(bytes, pos, b"null", JsonValue::Null),
        b'-' | b'0'..=b'9' => parse_number(bytes, input, pos),
        ch => Err(JsonError {
            message: format!("unexpected character '{}'", ch as char),
            position: *pos,
        }),
    }
}

fn skip_whitespace(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_literal(
    bytes: &[u8],
    pos: &mut usize,
    expected: &[u8],
    value: JsonValue,
) -> Result<JsonValue, JsonError> {
    let start = *pos;
    if bytes[start..].starts_with(expected) {
        *pos += expected.len();
        Ok(value)
    } else {
        Err(JsonError {
            message: format!("expected '{}'", std::str::from_utf8(expected).unwrap()),
            position: start,
        })
    }
}

fn parse_string(bytes: &[u8], input: &str, pos: &mut usize) -> Result<String, JsonError> {
    let start = *pos;
    if bytes[*pos] != b'"' {
        return Err(JsonError {
            message: "expected '\"'".into(),
            position: *pos,
        });
    }
    *pos += 1;

    let mut result = String::new();
    while *pos < bytes.len() {
        let ch = bytes[*pos];
        if ch == b'"' {
            *pos += 1;
            return Ok(result);
        }
        if ch == b'\\' {
            *pos += 1;
            if *pos >= bytes.len() {
                return Err(JsonError {
                    message: "unterminated escape sequence".into(),
                    position: *pos,
                });
            }
            match bytes[*pos] {
                b'"' => result.push('"'),
                b'\\' => result.push('\\'),
                b'/' => result.push('/'),
                b'n' => result.push('\n'),
                b'r' => result.push('\r'),
                b't' => result.push('\t'),
                b'b' => result.push('\u{0008}'),
                b'f' => result.push('\u{000C}'),
                b'u' => {
                    *pos += 1;
                    let cp = parse_hex4(bytes, pos)?;
                    // Handle surrogate pairs
                    if (0xD800..=0xDBFF).contains(&cp) {
                        // High surrogate — expect \uXXXX low surrogate
                        if *pos + 1 < bytes.len() && bytes[*pos] == b'\\' && bytes[*pos + 1] == b'u'
                        {
                            *pos += 2;
                            let low = parse_hex4(bytes, pos)?;
                            if (0xDC00..=0xDFFF).contains(&low) {
                                let code_point =
                                    0x10000 + ((cp as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
                                if let Some(c) = char::from_u32(code_point) {
                                    result.push(c);
                                } else {
                                    result.push('\u{FFFD}');
                                }
                            } else {
                                result.push('\u{FFFD}');
                            }
                        } else {
                            result.push('\u{FFFD}');
                        }
                    } else if (0xDC00..=0xDFFF).contains(&cp) {
                        result.push('\u{FFFD}');
                    } else if let Some(c) = char::from_u32(cp as u32) {
                        result.push(c);
                    } else {
                        result.push('\u{FFFD}');
                    }
                    continue; // parse_hex4 already advanced pos
                }
                _ => {
                    return Err(JsonError {
                        message: "invalid escape sequence".into(),
                        position: *pos,
                    });
                }
            }
            *pos += 1;
        } else if ch < 0x80 {
            result.push(ch as char);
            *pos += 1;
        } else {
            // Multi-byte UTF-8 — get the char from the str slice
            let remaining = &input[*pos..];
            if let Some(c) = remaining.chars().next() {
                result.push(c);
                *pos += c.len_utf8();
            } else {
                return Err(JsonError {
                    message: "invalid UTF-8".into(),
                    position: *pos,
                });
            }
        }
    }
    Err(JsonError {
        message: "unterminated string".into(),
        position: start,
    })
}

fn parse_hex4(bytes: &[u8], pos: &mut usize) -> Result<u16, JsonError> {
    if *pos + 4 > bytes.len() {
        return Err(JsonError {
            message: "incomplete unicode escape".into(),
            position: *pos,
        });
    }
    let hex_str = std::str::from_utf8(&bytes[*pos..*pos + 4]).map_err(|_| JsonError {
        message: "invalid hex digits".into(),
        position: *pos,
    })?;
    let val = u16::from_str_radix(hex_str, 16).map_err(|_| JsonError {
        message: "invalid hex digits".into(),
        position: *pos,
    })?;
    *pos += 4;
    Ok(val)
}

fn parse_number(bytes: &[u8], input: &str, pos: &mut usize) -> Result<JsonValue, JsonError> {
    let start = *pos;
    let mut is_float = false;

    // Optional minus
    if *pos < bytes.len() && bytes[*pos] == b'-' {
        *pos += 1;
    }

    // Integer part
    if *pos < bytes.len() && bytes[*pos] == b'0' {
        *pos += 1;
    } else if *pos < bytes.len() && bytes[*pos] >= b'1' && bytes[*pos] <= b'9' {
        *pos += 1;
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    } else {
        return Err(JsonError {
            message: "expected digit".into(),
            position: *pos,
        });
    }

    // Fractional part
    if *pos < bytes.len() && bytes[*pos] == b'.' {
        is_float = true;
        *pos += 1;
        if *pos >= bytes.len() || bytes[*pos] < b'0' || bytes[*pos] > b'9' {
            return Err(JsonError {
                message: "expected digit after '.'".into(),
                position: *pos,
            });
        }
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    }

    // Exponent
    if *pos < bytes.len() && (bytes[*pos] == b'e' || bytes[*pos] == b'E') {
        is_float = true;
        *pos += 1;
        if *pos < bytes.len() && (bytes[*pos] == b'+' || bytes[*pos] == b'-') {
            *pos += 1;
        }
        if *pos >= bytes.len() || bytes[*pos] < b'0' || bytes[*pos] > b'9' {
            return Err(JsonError {
                message: "expected digit in exponent".into(),
                position: *pos,
            });
        }
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    }

    let num_str = &input[start..*pos];
    if is_float {
        let f: f64 = num_str.parse().map_err(|_| JsonError {
            message: "invalid number".into(),
            position: start,
        })?;
        Ok(JsonValue::Number(JsonNumber::Float(f)))
    } else {
        match num_str.parse::<i64>() {
            Ok(n) => Ok(JsonValue::Number(JsonNumber::Int(n))),
            Err(_) => {
                // Overflow — try f64
                let f: f64 = num_str.parse().map_err(|_| JsonError {
                    message: "invalid number".into(),
                    position: start,
                })?;
                Ok(JsonValue::Number(JsonNumber::Float(f)))
            }
        }
    }
}

fn parse_array(bytes: &[u8], input: &str, pos: &mut usize) -> Result<JsonValue, JsonError> {
    let start = *pos;
    *pos += 1; // skip '['
    skip_whitespace(bytes, pos);

    let mut items = Vec::new();

    if *pos < bytes.len() && bytes[*pos] == b']' {
        *pos += 1;
        return Ok(JsonValue::Array(items));
    }

    loop {
        let val = parse_value(bytes, input, pos)?;
        items.push(val);
        skip_whitespace(bytes, pos);

        if *pos >= bytes.len() {
            return Err(JsonError {
                message: "unterminated array".into(),
                position: start,
            });
        }

        if bytes[*pos] == b']' {
            *pos += 1;
            return Ok(JsonValue::Array(items));
        }

        if bytes[*pos] != b',' {
            return Err(JsonError {
                message: "expected ',' or ']'".into(),
                position: *pos,
            });
        }
        *pos += 1;
    }
}

fn parse_object(bytes: &[u8], input: &str, pos: &mut usize) -> Result<JsonValue, JsonError> {
    let start = *pos;
    *pos += 1; // skip '{'
    skip_whitespace(bytes, pos);

    let mut pairs = Vec::new();

    if *pos < bytes.len() && bytes[*pos] == b'}' {
        *pos += 1;
        return Ok(JsonValue::Object(pairs));
    }

    loop {
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b'"' {
            return Err(JsonError {
                message: "expected string key".into(),
                position: *pos,
            });
        }
        let key = parse_string(bytes, input, pos)?;

        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b':' {
            return Err(JsonError {
                message: "expected ':'".into(),
                position: *pos,
            });
        }
        *pos += 1;

        let val = parse_value(bytes, input, pos)?;
        pairs.push((key, val));

        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() {
            return Err(JsonError {
                message: "unterminated object".into(),
                position: start,
            });
        }

        if bytes[*pos] == b'}' {
            *pos += 1;
            return Ok(JsonValue::Object(pairs));
        }

        if bytes[*pos] != b',' {
            return Err(JsonError {
                message: "expected ',' or '}'".into(),
                position: *pos,
            });
        }
        *pos += 1;
    }
}

// ── Display impl for convenient debug output ────────────────────────────────

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_primitives() {
        assert_eq!(parse("null").unwrap(), JsonValue::Null);
        assert_eq!(parse("true").unwrap(), JsonValue::Bool(true));
        assert_eq!(parse("false").unwrap(), JsonValue::Bool(false));
        assert_eq!(
            parse("42").unwrap(),
            JsonValue::Number(JsonNumber::Int(42))
        );
        assert_eq!(
            parse("-7").unwrap(),
            JsonValue::Number(JsonNumber::Int(-7))
        );
        assert_eq!(
            parse("3.14").unwrap(),
            JsonValue::Number(JsonNumber::Float(3.14))
        );
        assert_eq!(
            parse("\"hello\"").unwrap(),
            JsonValue::String("hello".into())
        );
    }

    #[test]
    fn test_parse_string_escapes() {
        let val = parse(r#""hello\nworld""#).unwrap();
        assert_eq!(val.as_str().unwrap(), "hello\nworld");

        let val = parse(r#""tab\there""#).unwrap();
        assert_eq!(val.as_str().unwrap(), "tab\there");

        let val = parse(r#""quote\"end""#).unwrap();
        assert_eq!(val.as_str().unwrap(), "quote\"end");

        let val = parse(r#""\u0041""#).unwrap();
        assert_eq!(val.as_str().unwrap(), "A");
    }

    #[test]
    fn test_parse_array() {
        let val = parse("[1, 2, 3]").unwrap();
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64().unwrap(), 1);
    }

    #[test]
    fn test_parse_object() {
        let val = parse(r#"{"a": 1, "b": "two"}"#).unwrap();
        assert_eq!(val.get("a").unwrap().as_i64().unwrap(), 1);
        assert_eq!(val.get("b").unwrap().as_str().unwrap(), "two");
    }

    #[test]
    fn test_builder() {
        let val = json_obj()
            .field_str("name", "sentinel")
            .field_i64("version", 1)
            .field_bool("active", true)
            .field_null("extra")
            .build();
        let s = val.to_json_string();
        let reparsed = parse(&s).unwrap();
        assert_eq!(reparsed.get("name").unwrap().as_str().unwrap(), "sentinel");
        assert_eq!(reparsed.get("version").unwrap().as_i64().unwrap(), 1);
    }

    #[test]
    fn test_roundtrip() {
        let input = r#"{"key":"value","num":42,"arr":[1,true,null],"nested":{"a":"b"}}"#;
        let val = parse(input).unwrap();
        let output = val.to_json_string();
        let reparsed = parse(&output).unwrap();
        assert_eq!(val, reparsed);
    }
}
