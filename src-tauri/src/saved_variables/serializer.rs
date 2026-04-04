use super::types::{SvTreeNode, SvValueType};

/// Serialize an `SvTreeNode` tree back to Lua source text.
/// The root node represents the file; each child is a top-level assignment.
pub fn serialize_to_lua(root: &SvTreeNode) -> String {
    let mut out = String::new();
    if let Some(children) = &root.children {
        for child in children {
            out.push_str(&child.key);
            out.push_str(" =\n");
            serialize_value(&mut out, child, 0);
            out.push('\n');
        }
    }
    out
}

fn serialize_value(out: &mut String, node: &SvTreeNode, depth: usize) {
    match node.value_type {
        SvValueType::Table => serialize_table(out, node, depth),
        SvValueType::String => {
            let s = node.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
            out.push('"');
            escape_lua_string(out, s);
            out.push('"');
        }
        SvValueType::Number => {
            if let Some(v) = &node.value {
                if let Some(n) = v.as_f64() {
                    // Use integer representation when possible
                    if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                        out.push_str(&(n as i64).to_string());
                    } else {
                        out.push_str(&format!("{}", n));
                    }
                } else {
                    out.push_str(&v.to_string());
                }
            }
        }
        SvValueType::Boolean => {
            if let Some(v) = &node.value {
                out.push_str(if v.as_bool().unwrap_or(false) {
                    "true"
                } else {
                    "false"
                });
            }
        }
        SvValueType::Nil => out.push_str("nil"),
    }
}

fn serialize_table(out: &mut String, node: &SvTreeNode, depth: usize) {
    let indent = "\t".repeat(depth);
    let child_indent = "\t".repeat(depth + 1);

    out.push_str("{\n");

    if let Some(children) = &node.children {
        for child in children {
            out.push_str(&child_indent);
            // Determine key format
            if is_numeric_key(&child.key) {
                out.push('[');
                out.push_str(&child.key);
                out.push_str("] = ");
            } else if is_identifier(&child.key) {
                out.push_str(&child.key);
                out.push_str(" = ");
            } else {
                out.push_str("[\"");
                escape_lua_string(out, &child.key);
                out.push_str("\"] = ");
            }
            serialize_value(out, child, depth + 1);
            out.push_str(",\n");
        }
    }

    out.push_str(&indent);
    out.push('}');
}

/// Check if a key is a valid Lua identifier (no quoting needed).
fn is_identifier(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Check if a key is a numeric index (e.g. "1", "42", "-3").
fn is_numeric_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let s = key.strip_prefix('-').unwrap_or(key);
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Escape a string for Lua double-quoted string literals.
fn escape_lua_string(out: &mut String, s: &str) {
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\x07' => out.push_str("\\a"),
            b'\x08' => out.push_str("\\b"),
            b'\x0B' => out.push_str("\\v"),
            b'\x0C' => out.push_str("\\f"),
            0x00..=0x1F => {
                // Other control characters: use zero-padded decimal escape
                // to avoid ambiguity when the next character is also a digit
                out.push_str(&format!("\\{:03}", b));
            }
            _ => out.push(b as char),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser;
    use super::*;

    #[test]
    fn serialize_simple_values() {
        let root = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(vec![SvTreeNode {
                key: "MyVar".into(),
                value_type: SvValueType::Table,
                value: None,
                children: Some(vec![
                    SvTreeNode {
                        key: "enabled".into(),
                        value_type: SvValueType::Boolean,
                        value: Some(serde_json::json!(true)),
                        children: None,
                    },
                    SvTreeNode {
                        key: "count".into(),
                        value_type: SvValueType::Number,
                        value: Some(serde_json::json!(42.0)),
                        children: None,
                    },
                    SvTreeNode {
                        key: "name".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("hello")),
                        children: None,
                    },
                ]),
            }]),
        };

        let lua = serialize_to_lua(&root);
        assert!(lua.contains("MyVar ="));
        assert!(lua.contains("enabled = true"));
        assert!(lua.contains("count = 42"));
        assert!(lua.contains("name = \"hello\""));
    }

    #[test]
    fn serialize_string_escapes() {
        let node = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                children: Some(vec![SvTreeNode {
                    key: "msg".into(),
                    value_type: SvValueType::String,
                    value: Some(serde_json::json!("line\nbreak\ttab\\slash\"quote")),
                    children: None,
                }]),
            }]),
        };
        let lua = serialize_to_lua(&node);
        assert!(lua.contains(r#"\"quote"#));
        assert!(lua.contains(r#"\n"#));
        assert!(lua.contains(r#"\t"#));
        assert!(lua.contains(r#"\\"#));
    }

    #[test]
    fn serialize_nil_value() {
        let node = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                children: Some(vec![SvTreeNode {
                    key: "nothing".into(),
                    value_type: SvValueType::Nil,
                    value: Some(serde_json::Value::Null),
                    children: None,
                }]),
            }]),
        };
        let lua = serialize_to_lua(&node);
        assert!(lua.contains("nothing = nil"));
    }

    #[test]
    fn serialize_numeric_keys() {
        let node = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                children: Some(vec![
                    SvTreeNode {
                        key: "1".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("first")),
                        children: None,
                    },
                    SvTreeNode {
                        key: "2".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("second")),
                        children: None,
                    },
                ]),
            }]),
        };
        let lua = serialize_to_lua(&node);
        assert!(lua.contains("[1] = \"first\""));
        assert!(lua.contains("[2] = \"second\""));
    }

    #[test]
    fn round_trip_simple() {
        let input = r#"MyAddon_SV =
{
	["Default"] =
	{
		["@Account"] =
		{
			["CharName"] =
			{
				["enabled"] = true,
				["version"] = 3,
				["name"] = "My Character",
			},
		},
	},
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_mixed_types() {
        let input = r#"TestVar =
{
	["boolVal"] = true,
	["numVal"] = 3.14,
	["strVal"] = "hello\nworld",
	["nilVal"] = nil,
	["nested"] =
	{
		["inner"] = 42,
	},
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_string_with_special_chars() {
        let input = r#"Var =
{
	["msg"] = "tabs\there\nlines\nand\\slashes\"quotes",
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_deeply_nested() {
        let input = r#"Deep =
{
	["a"] =
	{
		["b"] =
		{
			["c"] =
			{
				["d"] = 1,
			},
		},
	},
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_array_table() {
        let input = r#"Arr =
{
	[1] = "first",
	[2] = "second",
	[3] = "third",
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_real_eso_structure() {
        let input = r#"Srendarr_SV =
{
	["Default"] =
	{
		["@MyAccount"] =
		{
			["$AccountWide"] =
			{
				["bars"] =
				{
					["bar1"] =
					{
						["enabled"] = true,
						["x"] = 100,
						["y"] = 200,
						["scale"] = 1,
						["name"] = "Main Bar",
					},
				},
				["general"] =
				{
					["locked"] = false,
					["showOutOfCombat"] = true,
					["combatOnly"] = false,
				},
			},
			["MyChar^NA Megaserver"] =
			{
				["bars"] =
				{
					["bar1"] =
					{
						["enabled"] = true,
						["x"] = 150,
						["y"] = 250,
					},
				},
			},
		},
	},
}
"#;
        let tree = parser::parse_sv_file(input, "Srendarr.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "Srendarr.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_multiple_top_level_vars() {
        let input = r#"Var1 =
{
	["key1"] = "value1",
}
Var2 =
{
	["key2"] = 42,
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_empty_tables() {
        let input = r#"Empty =
{
	["sub"] =
	{
	},
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn round_trip_identifier_keys() {
        let input = r#"Var =
{
	enabled = true,
	level = 10,
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        let tree2 = parser::parse_sv_file(&output, "test.lua").unwrap();
        assert_eq!(tree, tree2);
    }

    #[test]
    fn is_identifier_tests() {
        assert!(is_identifier("enabled"));
        assert!(is_identifier("_private"));
        assert!(is_identifier("myVar123"));
        assert!(!is_identifier("123abc"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("my-var"));
        assert!(!is_identifier("my var"));
    }

    #[test]
    fn is_numeric_key_tests() {
        assert!(is_numeric_key("1"));
        assert!(is_numeric_key("42"));
        assert!(is_numeric_key("-3"));
        assert!(!is_numeric_key("abc"));
        assert!(!is_numeric_key(""));
        assert!(!is_numeric_key("1.5"));
    }
}
