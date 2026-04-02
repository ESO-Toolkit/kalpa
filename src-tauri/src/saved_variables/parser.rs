use super::types::SvTreeNode;

/// Simple recursive descent parser for ESO SavedVariables Lua files.
/// Only handles the subset of Lua that ESO actually generates.
pub fn parse_lua_value(chars: &[u8], pos: &mut usize) -> Result<SvTreeNode, String> {
    skip_whitespace_and_comments(chars, pos);

    if *pos >= chars.len() {
        return Err("Unexpected end of input".to_string());
    }

    match chars[*pos] {
        b'{' => parse_lua_table(chars, pos),
        b'"' | b'\'' => parse_lua_quoted_string(chars, pos),
        b'[' if *pos + 1 < chars.len() && (chars[*pos + 1] == b'[' || chars[*pos + 1] == b'=') => {
            parse_lua_long_string(chars, pos)
        }
        b't' if chars[*pos..].starts_with(b"true") => {
            *pos += 4;
            Ok(SvTreeNode {
                key: String::new(),
                value_type: "boolean".to_string(),
                value: Some(serde_json::Value::Bool(true)),
                children: None,
            })
        }
        b'f' if chars[*pos..].starts_with(b"false") => {
            *pos += 5;
            Ok(SvTreeNode {
                key: String::new(),
                value_type: "boolean".to_string(),
                value: Some(serde_json::Value::Bool(false)),
                children: None,
            })
        }
        b'n' if chars[*pos..].starts_with(b"nil") => {
            *pos += 3;
            Ok(SvTreeNode {
                key: String::new(),
                value_type: "nil".to_string(),
                value: Some(serde_json::Value::Null),
                children: None,
            })
        }
        b'-' | b'0'..=b'9' => parse_lua_number(chars, pos),
        b'.' if *pos + 1 < chars.len() && chars[*pos + 1].is_ascii_digit() => {
            parse_lua_number(chars, pos)
        }
        _ => Err(format!(
            "Unexpected character '{}' at position {}",
            chars[*pos] as char, *pos
        )),
    }
}

pub fn skip_whitespace_and_comments(chars: &[u8], pos: &mut usize) {
    while *pos < chars.len() {
        match chars[*pos] {
            b' ' | b'\t' | b'\r' | b'\n' => *pos += 1,
            b'-' if *pos + 1 < chars.len() && chars[*pos + 1] == b'-' => {
                *pos += 2;
                // Block comment: --[[ ... ]] or --[=[ ... ]=] etc.
                if *pos < chars.len() && chars[*pos] == b'[' {
                    if let Some(end) = skip_long_bracket(chars, *pos) {
                        *pos = end;
                        continue;
                    }
                }
                // Line comment: skip to end of line
                while *pos < chars.len() && chars[*pos] != b'\n' {
                    *pos += 1;
                }
            }
            _ => break,
        }
    }
}

/// Try to skip a long bracket starting at `pos` (which should point at `[`).
/// Returns the position just past the closing bracket, or None if not a long bracket.
pub fn skip_long_bracket(chars: &[u8], pos: usize) -> Option<usize> {
    if pos >= chars.len() || chars[pos] != b'[' {
        return None;
    }
    let mut i = pos + 1;
    let mut level = 0usize;
    while i < chars.len() && chars[i] == b'=' {
        level += 1;
        i += 1;
    }
    if i >= chars.len() || chars[i] != b'[' {
        return None;
    }
    i += 1; // skip second [

    // Build closing pattern: ] + =*level + ]
    let mut close = vec![b']'];
    close.extend(std::iter::repeat_n(b'=', level));
    close.push(b']');

    while i + close.len() <= chars.len() {
        if &chars[i..i + close.len()] == close.as_slice() {
            return Some(i + close.len());
        }
        i += 1;
    }
    None
}

fn parse_lua_quoted_string(chars: &[u8], pos: &mut usize) -> Result<SvTreeNode, String> {
    let quote = chars[*pos];
    *pos += 1; // skip opening quote
    let mut out = Vec::new();
    while *pos < chars.len() && chars[*pos] != quote {
        if chars[*pos] == b'\\' {
            *pos += 1;
            if *pos >= chars.len() {
                return Err("Unterminated escape in string".to_string());
            }
            match chars[*pos] {
                b'a' => out.push(b'\x07'),
                b'b' => out.push(b'\x08'),
                b'f' => out.push(b'\x0C'),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'v' => out.push(b'\x0B'),
                b'\\' => out.push(b'\\'),
                b'\'' => out.push(b'\''),
                b'"' => out.push(b'"'),
                b'\n' | b'\r' => out.push(b'\n'), // escaped newline
                d @ b'0'..=b'9' => {
                    // \ddd decimal escape (up to 3 digits)
                    let mut val: u16 = (d - b'0') as u16;
                    for _ in 0..2 {
                        if *pos + 1 < chars.len() && chars[*pos + 1].is_ascii_digit() {
                            *pos += 1;
                            val = val * 10 + (chars[*pos] - b'0') as u16;
                        } else {
                            break;
                        }
                    }
                    if val > 255 {
                        return Err(format!("Decimal escape \\{} out of range", val));
                    }
                    out.push(val as u8);
                }
                other => {
                    // Unknown escape: keep as-is (Lua would error, but be lenient)
                    out.push(b'\\');
                    out.push(other);
                }
            }
        } else {
            out.push(chars[*pos]);
        }
        *pos += 1;
    }
    if *pos >= chars.len() {
        return Err("Unterminated string".to_string());
    }
    *pos += 1; // skip closing quote
    let s = String::from_utf8_lossy(&out).to_string();
    Ok(SvTreeNode {
        key: String::new(),
        value_type: "string".to_string(),
        value: Some(serde_json::Value::String(s)),
        children: None,
    })
}

/// Parse Lua long bracket strings: `[[...]]`, `[=[...]=]`, `[==[...]==]`, etc.
fn parse_lua_long_string(chars: &[u8], pos: &mut usize) -> Result<SvTreeNode, String> {
    let start = *pos;
    *pos += 1; // skip first [
    let mut level = 0usize;
    while *pos < chars.len() && chars[*pos] == b'=' {
        level += 1;
        *pos += 1;
    }
    if *pos >= chars.len() || chars[*pos] != b'[' {
        *pos = start;
        return Err(format!("Invalid long bracket string at position {}", start));
    }
    *pos += 1; // skip second [

    // Build the closing pattern: `]` + `=` * level + `]`
    let mut close_pattern = vec![b']'];
    close_pattern.extend(std::iter::repeat_n(b'=', level));
    close_pattern.push(b']');

    let content_start = *pos;
    while *pos + close_pattern.len() <= chars.len() {
        if &chars[*pos..*pos + close_pattern.len()] == close_pattern.as_slice() {
            let s = String::from_utf8_lossy(&chars[content_start..*pos]).to_string();
            *pos += close_pattern.len();
            return Ok(SvTreeNode {
                key: String::new(),
                value_type: "string".to_string(),
                value: Some(serde_json::Value::String(s)),
                children: None,
            });
        }
        *pos += 1;
    }
    Err(format!(
        "Unterminated long bracket string starting at position {}",
        start
    ))
}

fn parse_lua_number(chars: &[u8], pos: &mut usize) -> Result<SvTreeNode, String> {
    let start = *pos;
    if *pos < chars.len() && chars[*pos] == b'-' {
        *pos += 1;
    }
    // Handle hex literals: 0x or 0X
    if *pos + 1 < chars.len()
        && chars[*pos] == b'0'
        && (chars[*pos + 1] == b'x' || chars[*pos + 1] == b'X')
    {
        *pos += 2; // skip 0x
        while *pos < chars.len() && chars[*pos].is_ascii_hexdigit() {
            *pos += 1;
        }
    } else {
        while *pos < chars.len()
            && (chars[*pos].is_ascii_digit()
                || chars[*pos] == b'.'
                || chars[*pos] == b'e'
                || chars[*pos] == b'E'
                || chars[*pos] == b'+'
                || chars[*pos] == b'-')
        {
            // Avoid consuming a '-' that isn't part of scientific notation
            if (chars[*pos] == b'+' || chars[*pos] == b'-') && *pos > start + 1 {
                let prev = chars[*pos - 1];
                if prev != b'e' && prev != b'E' {
                    break;
                }
            }
            *pos += 1;
        }
    }
    let num_str = String::from_utf8_lossy(&chars[start..*pos]).to_string();
    let value = if num_str.contains('x') || num_str.contains('X') {
        let (negative, hex_part) = if num_str.starts_with('-') {
            (true, &num_str[3..]) // skip -0x
        } else {
            (false, &num_str[2..]) // skip 0x
        };
        let n = i64::from_str_radix(hex_part, 16)
            .map(|v| if negative { -v } else { v })
            .map_err(|_| format!("Invalid hex number: {}", num_str))?;
        serde_json::Value::Number(serde_json::Number::from(n))
    } else if let Ok(n) = num_str.parse::<f64>() {
        serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    } else {
        return Err(format!("Invalid number: {}", num_str));
    };
    Ok(SvTreeNode {
        key: String::new(),
        value_type: "number".to_string(),
        value: Some(value),
        children: None,
    })
}

fn parse_lua_table(chars: &[u8], pos: &mut usize) -> Result<SvTreeNode, String> {
    *pos += 1; // skip {
    let mut children: Vec<SvTreeNode> = Vec::new();
    let mut index = 1u32;

    loop {
        skip_whitespace_and_comments(chars, pos);
        if *pos >= chars.len() {
            return Err("Unterminated table".to_string());
        }
        if chars[*pos] == b'}' {
            *pos += 1;
            break;
        }

        // Try to parse key = value or ["key"] = value
        let key = parse_table_key(chars, pos)?;

        let mut child = parse_lua_value(chars, pos)?;
        child.key = key.unwrap_or_else(|| {
            let k = index.to_string();
            index += 1;
            k
        });

        children.push(child);

        // Skip optional comma
        skip_whitespace_and_comments(chars, pos);
        if *pos < chars.len() && chars[*pos] == b',' {
            *pos += 1;
        }
    }

    Ok(SvTreeNode {
        key: String::new(),
        value_type: "table".to_string(),
        value: None,
        children: Some(children),
    })
}

/// Parse table key: `["string"]` or `[number]` or `identifier` followed by `=`
/// Returns None for array entries (no key).
fn parse_table_key(chars: &[u8], pos: &mut usize) -> Result<Option<String>, String> {
    skip_whitespace_and_comments(chars, pos);

    if *pos >= chars.len() {
        return Ok(None);
    }

    let saved = *pos;

    if chars[*pos] == b'[' {
        *pos += 1;
        skip_whitespace_and_comments(chars, pos);
        if *pos >= chars.len() {
            *pos = saved;
            return Ok(None);
        }

        let key = if chars[*pos] == b'"' {
            *pos += 1;
            let start = *pos;
            while *pos < chars.len() && chars[*pos] != b'"' {
                if chars[*pos] == b'\\' {
                    *pos += 1;
                }
                *pos += 1;
            }
            if *pos >= chars.len() {
                *pos = saved;
                return Ok(None);
            }
            let k = String::from_utf8_lossy(&chars[start..*pos]).to_string();
            *pos += 1; // skip "
            k
        } else if chars[*pos].is_ascii_digit() || chars[*pos] == b'-' {
            let start = *pos;
            if chars[*pos] == b'-' {
                *pos += 1;
            }
            while *pos < chars.len() && chars[*pos].is_ascii_digit() {
                *pos += 1;
            }
            String::from_utf8_lossy(&chars[start..*pos]).to_string()
        } else {
            *pos = saved;
            return Ok(None);
        };

        skip_whitespace_and_comments(chars, pos);
        if *pos < chars.len() && chars[*pos] == b']' {
            *pos += 1;
        } else {
            *pos = saved;
            return Ok(None);
        }

        skip_whitespace_and_comments(chars, pos);
        if *pos < chars.len() && chars[*pos] == b'=' {
            *pos += 1;
            return Ok(Some(key));
        }

        *pos = saved;
        return Ok(None);
    }

    // Try identifier = value
    if chars[*pos].is_ascii_alphabetic() || chars[*pos] == b'_' {
        let start = *pos;
        while *pos < chars.len() && (chars[*pos].is_ascii_alphanumeric() || chars[*pos] == b'_') {
            *pos += 1;
        }
        let ident = String::from_utf8_lossy(&chars[start..*pos]).to_string();
        skip_whitespace_and_comments(chars, pos);
        if *pos < chars.len() && chars[*pos] == b'=' {
            *pos += 1;
            return Ok(Some(ident));
        }
        // Not a key=value, backtrack
        *pos = saved;
        return Ok(None);
    }

    Ok(None)
}

/// Parse a full SavedVariables file into a virtual root node.
pub fn parse_sv_file(content: &str, file_name: &str) -> Result<SvTreeNode, String> {
    let bytes = content.as_bytes();
    let mut pos = 0;
    let mut children: Vec<SvTreeNode> = Vec::new();

    while pos < bytes.len() {
        skip_whitespace_and_comments(bytes, &mut pos);
        if pos >= bytes.len() {
            break;
        }

        // Expect: identifier = { ... }
        if bytes[pos].is_ascii_alphabetic() || bytes[pos] == b'_' {
            let start = pos;
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let var_name = String::from_utf8_lossy(&bytes[start..pos]).to_string();

            skip_whitespace_and_comments(bytes, &mut pos);
            if pos < bytes.len() && bytes[pos] == b'=' {
                pos += 1;
                match parse_lua_value(bytes, &mut pos) {
                    Ok(mut node) => {
                        node.key = var_name;
                        children.push(node);
                    }
                    Err(e) => {
                        return Err(format!("Parse error in {}: {}", file_name, e));
                    }
                }
            } else {
                // Skip unknown content
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }

    Ok(SvTreeNode {
        key: file_name.to_string(),
        value_type: "table".to_string(),
        value: None,
        children: Some(children),
    })
}

/// Skip past a quoted string or long bracket string starting at position `i`.
/// Returns the position after the closing delimiter.
pub fn skip_lua_string(bytes: &[u8], i: usize) -> usize {
    match bytes[i] {
        b'"' | b'\'' => {
            let quote = bytes[i];
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != quote {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j < bytes.len() {
                j + 1
            } else {
                j
            }
        }
        b'[' => {
            if let Some(end) = skip_long_bracket(bytes, i) {
                end
            } else {
                i + 1
            }
        }
        _ => i + 1,
    }
}

/// Skip a comment starting at position `i` (which should point to the first `-`).
/// Handles both line comments and block comments (--[[ ]]).
pub fn skip_lua_comment(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 2; // skip --
                       // Check for block comment
    if j < bytes.len() && bytes[j] == b'[' {
        if let Some(end) = skip_long_bracket(bytes, j) {
            return end;
        }
    }
    // Line comment
    while j < bytes.len() && bytes[j] != b'\n' {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_value(input: &str) -> Result<SvTreeNode, String> {
        let bytes = input.as_bytes();
        let mut pos = 0;
        parse_lua_value(bytes, &mut pos)
    }

    #[test]
    fn lua_parse_string_simple() {
        let node = parse_value(r#""hello world""#).unwrap();
        assert_eq!(node.value_type, "string");
        assert_eq!(node.value, Some(serde_json::json!("hello world")));
    }

    #[test]
    fn lua_parse_string_escapes() {
        let node = parse_value(r#""line\nbreak\ttab\\slash""#).unwrap();
        assert_eq!(
            node.value,
            Some(serde_json::json!("line\nbreak\ttab\\slash"))
        );
    }

    #[test]
    fn lua_parse_string_decimal_escape() {
        let node = parse_value(r#""\72\101\108""#).unwrap();
        assert_eq!(node.value, Some(serde_json::json!("Hel")));
    }

    #[test]
    fn lua_parse_string_single_quotes() {
        let node = parse_value("'single'").unwrap();
        assert_eq!(node.value, Some(serde_json::json!("single")));
    }

    #[test]
    fn lua_parse_long_string() {
        let node = parse_value("[[long string content]]").unwrap();
        assert_eq!(node.value_type, "string");
        assert_eq!(node.value, Some(serde_json::json!("long string content")));
    }

    #[test]
    fn lua_parse_long_string_with_equals() {
        let node = parse_value("[=[contains ]] brackets]=]").unwrap();
        assert_eq!(node.value, Some(serde_json::json!("contains ]] brackets")));
    }

    #[test]
    fn lua_parse_integer() {
        let node = parse_value("42").unwrap();
        assert_eq!(node.value_type, "number");
        assert_eq!(node.value, Some(serde_json::json!(42.0)));
    }

    #[test]
    fn lua_parse_negative_integer() {
        let node = parse_value("-7").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(-7.0)));
    }

    #[test]
    fn lua_parse_float() {
        let node = parse_value("3.14").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(3.14)));
    }

    #[test]
    fn lua_parse_scientific_notation() {
        let node = parse_value("1.5e3").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(1500.0)));
    }

    #[test]
    fn lua_parse_hex_number() {
        let node = parse_value("0xFF").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(255)));
    }

    #[test]
    fn lua_parse_negative_hex() {
        let node = parse_value("-0x10").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(-16)));
    }

    #[test]
    fn lua_parse_true() {
        let node = parse_value("true").unwrap();
        assert_eq!(node.value_type, "boolean");
        assert_eq!(node.value, Some(serde_json::json!(true)));
    }

    #[test]
    fn lua_parse_false() {
        let node = parse_value("false").unwrap();
        assert_eq!(node.value_type, "boolean");
        assert_eq!(node.value, Some(serde_json::json!(false)));
    }

    #[test]
    fn lua_parse_nil() {
        let node = parse_value("nil").unwrap();
        assert_eq!(node.value_type, "nil");
        assert_eq!(node.value, Some(serde_json::Value::Null));
    }

    #[test]
    fn lua_parse_empty_table() {
        let node = parse_value("{}").unwrap();
        assert_eq!(node.value_type, "table");
        assert!(node.children.as_ref().unwrap().is_empty());
    }

    #[test]
    fn lua_parse_table_with_string_keys() {
        let node = parse_value(r#"{ ["name"] = "test", ["count"] = 5 }"#).unwrap();
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key, "name");
        assert_eq!(children[0].value, Some(serde_json::json!("test")));
        assert_eq!(children[1].key, "count");
        assert_eq!(children[1].value, Some(serde_json::json!(5.0)));
    }

    #[test]
    fn lua_parse_table_with_identifier_keys() {
        let node = parse_value("{ enabled = true, level = 10 }").unwrap();
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key, "enabled");
        assert_eq!(children[0].value, Some(serde_json::json!(true)));
        assert_eq!(children[1].key, "level");
        assert_eq!(children[1].value, Some(serde_json::json!(10.0)));
    }

    #[test]
    fn lua_parse_table_with_numeric_keys() {
        let node = parse_value("{ [1] = \"a\", [2] = \"b\" }").unwrap();
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key, "1");
        assert_eq!(children[1].key, "2");
    }

    #[test]
    fn lua_parse_array_style_table() {
        let node = parse_value("{ \"first\", \"second\", \"third\" }").unwrap();
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].key, "1");
        assert_eq!(children[1].key, "2");
        assert_eq!(children[2].key, "3");
    }

    #[test]
    fn lua_parse_nested_tables() {
        let input = r#"{ ["outer"] = { ["inner"] = 42 } }"#;
        let node = parse_value(input).unwrap();
        let outer = &node.children.as_ref().unwrap()[0];
        assert_eq!(outer.key, "outer");
        assert_eq!(outer.value_type, "table");
        let inner = &outer.children.as_ref().unwrap()[0];
        assert_eq!(inner.key, "inner");
        assert_eq!(inner.value, Some(serde_json::json!(42.0)));
    }

    #[test]
    fn lua_parse_with_line_comments() {
        let input = "-- this is a comment\n42";
        let node = parse_value(input).unwrap();
        assert_eq!(node.value, Some(serde_json::json!(42.0)));
    }

    #[test]
    fn lua_parse_with_block_comments() {
        let input = "--[[ block comment ]] 42";
        let node = parse_value(input).unwrap();
        assert_eq!(node.value, Some(serde_json::json!(42.0)));
    }

    #[test]
    fn lua_parse_unterminated_string_errors() {
        assert!(parse_value("\"unterminated").is_err());
    }

    #[test]
    fn lua_parse_unterminated_table_errors() {
        assert!(parse_value("{ \"a\", \"b\"").is_err());
    }

    #[test]
    fn lua_parse_empty_input_errors() {
        assert!(parse_value("").is_err());
    }

    #[test]
    fn lua_parse_eso_style_savedvar() {
        let input = r#"{
	["Default"] =
	{
		["@AccountName"] =
		{
			["CharName^NA Megaserver"] =
			{
				["enabled"] = true,
				["version"] = 3,
				["name"] = "My Character",
			},
		},
	},
}"#;
        let node = parse_value(input).unwrap();
        let default = &node.children.as_ref().unwrap()[0];
        assert_eq!(default.key, "Default");
        let account = &default.children.as_ref().unwrap()[0];
        assert_eq!(account.key, "@AccountName");
        let char = &account.children.as_ref().unwrap()[0];
        assert_eq!(char.key, "CharName^NA Megaserver");
        let children = char.children.as_ref().unwrap();
        assert_eq!(children[0].key, "enabled");
        assert_eq!(children[0].value, Some(serde_json::json!(true)));
        assert_eq!(children[1].key, "version");
        assert_eq!(children[1].value, Some(serde_json::json!(3.0)));
        assert_eq!(children[2].key, "name");
        assert_eq!(children[2].value, Some(serde_json::json!("My Character")));
    }

    #[test]
    fn lua_parse_leading_dot_number() {
        let node = parse_value(".5").unwrap();
        assert_eq!(node.value, Some(serde_json::json!(0.5)));
    }

    #[test]
    fn lua_parse_string_with_bell_escape() {
        let node = parse_value(r#""\a""#).unwrap();
        let s = node.value.unwrap();
        assert_eq!(s.as_str().unwrap().as_bytes()[0], 0x07);
    }
}
