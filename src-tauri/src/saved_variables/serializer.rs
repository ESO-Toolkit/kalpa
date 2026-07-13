use super::types::{SvTreeNode, SvValueType};
use std::fmt::{self, Write};

/// Serialize an `SvTreeNode` tree back to Lua source text.
/// The root node represents the file; each child is a top-level assignment.
pub fn serialize_to_lua(root: &SvTreeNode) -> String {
    let mut out = String::new();
    serialize_root(&mut out, root);
    out
}

/// Byte length of what [`serialize_to_lua`] would produce for `root`, computed
/// by running the exact same serialization logic into a counting sink instead
/// of materializing the `String`.
///
/// This is byte-identical to `serialize_to_lua(root).len()` (both drive
/// [`serialize_root`]) but never allocates the output — used by the scrubber's
/// byte-accounting, where whole subtrees would otherwise be serialized to
/// throwaway `String`s just to measure them.
pub fn serialized_len(root: &SvTreeNode) -> usize {
    let mut counter = ByteCounter::default();
    serialize_root(&mut counter, root);
    counter.0
}

/// A `fmt::Write` sink that discards its input and only tallies the byte length.
/// `write_str` sums `s.len()`, and the default `write_char` routes a char
/// through `write_str` after UTF-8 encoding, so the tally matches `String`'s
/// own byte growth exactly (including `push(b as char)` for `0x80..=0xFF`,
/// which `String` stores as two UTF-8 bytes).
#[derive(Default)]
struct ByteCounter(usize);

impl fmt::Write for ByteCounter {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

/// Writes to `String` and to [`ByteCounter`] are both infallible; this helper
/// keeps the serializer body free of `unwrap`/`?` noise while remaining generic
/// over the sink.
#[inline]
fn w<W: Write>(out: &mut W, s: &str) {
    let _ = out.write_str(s);
}

fn serialize_root<W: Write>(out: &mut W, root: &SvTreeNode) {
    if let Some(children) = &root.children {
        for child in children {
            w(out, &child.key);
            w(out, " =\n");
            serialize_value(out, child, 0);
            w(out, "\n");
        }
    }
}

fn serialize_value<W: Write>(out: &mut W, node: &SvTreeNode, depth: usize) {
    match node.value_type {
        SvValueType::Table => serialize_table(out, node, depth),
        SvValueType::String => {
            if let Some(raw) = &node.raw_lua_value {
                // Pre-escaped content for non-UTF8 strings: write verbatim
                w(out, "\"");
                w(out, raw);
                w(out, "\"");
            } else {
                let s = node.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
                w(out, "\"");
                escape_lua_string(out, s);
                w(out, "\"");
            }
        }
        SvValueType::Number => {
            if let Some(v) = &node.value {
                if let Some(n) = v.as_f64() {
                    if n.is_nan() || n.is_infinite() {
                        w(out, "0");
                    } else if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                        let _ = write!(out, "{}", n as i64);
                    } else {
                        let _ = write!(out, "{n}");
                    }
                } else {
                    let _ = write!(out, "{v}");
                }
            }
        }
        SvValueType::Boolean => {
            if let Some(v) = &node.value {
                w(
                    out,
                    if v.as_bool().unwrap_or(false) {
                        "true"
                    } else {
                        "false"
                    },
                );
            }
        }
        SvValueType::Nil => w(out, "nil"),
    }
}

/// Push `depth` tab characters into `out`. Replaces the previous
/// `"\t".repeat(depth)` temporaries (two per table node).
#[inline]
fn push_indent<W: Write>(out: &mut W, depth: usize) {
    for _ in 0..depth {
        w(out, "\t");
    }
}

fn serialize_table<W: Write>(out: &mut W, node: &SvTreeNode, depth: usize) {
    if depth >= 512 {
        w(out, "{}");
        return;
    }

    w(out, "{\n");

    if let Some(children) = &node.children {
        for child in children {
            push_indent(out, depth + 1);
            // Determine key format. Numeric keys use the bare `[N] =` array
            // form; all other (string) keys use the bracketed-quoted
            // `["key"] =` form. ESO's own SavedVariables writer always emits
            // the quoted form for string keys even when they are valid Lua
            // identifiers, and kalpa features that scan the game format
            // (character-key extraction in io.rs, copy-profile in profile.rs)
            // depend on that. Emitting bare identifiers here would make
            // identifier-like character names vanish from those features.
            if is_numeric_key(&child.key) {
                w(out, "[");
                w(out, &child.key);
                w(out, "] = ");
            } else {
                w(out, "[\"");
                escape_lua_string(out, &child.key);
                w(out, "\"] = ");
            }
            serialize_value(out, child, depth + 1);
            w(out, ",\n");
        }
    }

    push_indent(out, depth);
    w(out, "}");
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
fn escape_lua_string<W: Write>(out: &mut W, s: &str) {
    for b in s.bytes() {
        match b {
            b'\\' => w(out, "\\\\"),
            b'"' => w(out, "\\\""),
            b'\n' => w(out, "\\n"),
            b'\r' => w(out, "\\r"),
            b'\t' => w(out, "\\t"),
            b'\x07' => w(out, "\\a"),
            b'\x08' => w(out, "\\b"),
            b'\x0B' => w(out, "\\v"),
            b'\x0C' => w(out, "\\f"),
            0x00..=0x1F => {
                // Other control characters: use zero-padded decimal escape
                // to avoid ambiguity when the next character is also a digit
                let _ = write!(out, "\\{b:03}");
            }
            // Bytes >= 0x20: `b as char` matches the original `String::push`,
            // which stores 0x80..=0xFF as two UTF-8 bytes — the counting sink
            // tallies the same length via `write_char`.
            _ => {
                let _ = out.write_char(b as char);
            }
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
                raw_lua_value: None,
                children: Some(vec![
                    SvTreeNode {
                        key: "enabled".into(),
                        value_type: SvValueType::Boolean,
                        value: Some(serde_json::json!(true)),
                        children: None,
                        raw_lua_value: None,
                    },
                    SvTreeNode {
                        key: "count".into(),
                        value_type: SvValueType::Number,
                        value: Some(serde_json::json!(42.0)),
                        children: None,
                        raw_lua_value: None,
                    },
                    SvTreeNode {
                        key: "name".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("hello")),
                        children: None,
                        raw_lua_value: None,
                    },
                ]),
            }]),
            raw_lua_value: None,
        };

        let lua = serialize_to_lua(&root);
        // Top-level variable names stay bare identifiers.
        assert!(lua.contains("MyVar ="));
        // Nested string keys use the bracketed-quoted game format.
        assert!(lua.contains("[\"enabled\"] = true"));
        assert!(lua.contains("[\"count\"] = 42"));
        assert!(lua.contains("[\"name\"] = \"hello\""));
    }

    #[test]
    fn serialize_string_escapes() {
        let node = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            raw_lua_value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                raw_lua_value: None,
                children: Some(vec![SvTreeNode {
                    key: "msg".into(),
                    value_type: SvValueType::String,
                    value: Some(serde_json::json!("line\nbreak\ttab\\slash\"quote")),
                    children: None,
                    raw_lua_value: None,
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
            raw_lua_value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                raw_lua_value: None,
                children: Some(vec![SvTreeNode {
                    key: "nothing".into(),
                    value_type: SvValueType::Nil,
                    value: Some(serde_json::Value::Null),
                    children: None,
                    raw_lua_value: None,
                }]),
            }]),
        };
        let lua = serialize_to_lua(&node);
        assert!(lua.contains("[\"nothing\"] = nil"));
    }

    #[test]
    fn serialize_numeric_keys() {
        let node = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            raw_lua_value: None,
            children: Some(vec![SvTreeNode {
                key: "Var".into(),
                value_type: SvValueType::Table,
                value: None,
                raw_lua_value: None,
                children: Some(vec![
                    SvTreeNode {
                        key: "1".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("first")),
                        children: None,
                        raw_lua_value: None,
                    },
                    SvTreeNode {
                        key: "2".into(),
                        value_type: SvValueType::String,
                        value: Some(serde_json::json!("second")),
                        children: None,
                        raw_lua_value: None,
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
    fn identifier_safe_keys_serialize_bracket_quoted() {
        // Even keys that are valid Lua identifiers must serialize as
        // ["key"] = ... so the regex-based tools that scan these files
        // (extract_character_keys, copy_sv_profile) keep matching them.
        let input = r#"Var =
{
	enabled = true,
	level = 10,
}
"#;
        let tree = parser::parse_sv_file(input, "test.lua").unwrap();
        let output = serialize_to_lua(&tree);
        assert!(output.contains("[\"enabled\"] = true"));
        assert!(output.contains("[\"level\"] = 10"));
        assert!(!output.contains("enabled = true"));
        assert!(!output.contains("level = 10"));
    }

    #[test]
    fn nested_identifier_key_serializes_bracketed() {
        // Even though "Baelthor" is a valid Lua identifier, a nested string key
        // must be emitted in ESO's `["Name"] =` game format so that
        // character-key extraction (io.rs) and copy-profile (profile.rs), which
        // scan for that format, keep working after a save through the SV editor.
        let root = SvTreeNode {
            key: "test.lua".into(),
            value_type: SvValueType::Table,
            value: None,
            raw_lua_value: None,
            children: Some(vec![SvTreeNode {
                key: "MyAddon_SV".into(),
                value_type: SvValueType::Table,
                value: None,
                raw_lua_value: None,
                children: Some(vec![SvTreeNode {
                    key: "Baelthor".into(),
                    value_type: SvValueType::Table,
                    value: None,
                    raw_lua_value: None,
                    children: Some(vec![SvTreeNode {
                        key: "level".into(),
                        value_type: SvValueType::Number,
                        value: Some(serde_json::json!(50.0)),
                        children: None,
                        raw_lua_value: None,
                    }]),
                }]),
            }]),
        };

        let lua = serialize_to_lua(&root);
        // Top-level variable name stays a bare identifier.
        assert!(lua.contains("MyAddon_SV ="));
        // Identifier-like nested key round-trips in bracketed-quoted form.
        assert!(lua.contains("[\"Baelthor\"] ="));
        assert!(!lua.contains("Baelthor ="));

        // And extraction sees it: place Baelthor at character-key depth (3).
        let wrapped = "MyAddon_SV =\n{\n\t[\"Default\"] =\n\t{\n\t\t[\"@Acct\"] =\n\t\t{\n\t\t\t[\"Baelthor\"] =\n\t\t\t{\n\t\t\t\t[\"level\"] = 50,\n\t\t\t},\n\t\t},\n\t},\n}\n";
        let keys = super::super::io::extract_character_keys(wrapped);
        assert!(keys.contains(&"Baelthor".to_string()));
    }

    #[test]
    fn serialized_len_matches_serialize_to_lua_len() {
        // A nontrivial tree: nested tables, string escapes, numbers (int, float,
        // negative), booleans, nil, numeric array keys, a control-char escape,
        // and a high byte (0xE9) that String stores as two UTF-8 bytes.
        let input = concat!(
            "Complex_SV =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"@Acct\"] =\n\t\t{\n",
            "\t\t\t[\"Char\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"enabled\"] = true,\n",
            "\t\t\t\t[\"disabled\"] = false,\n",
            "\t\t\t\t[\"nothing\"] = nil,\n",
            "\t\t\t\t[\"count\"] = 42,\n",
            "\t\t\t\t[\"ratio\"] = 3.14,\n",
            "\t\t\t\t[\"neg\"] = -7,\n",
            "\t\t\t\t[\"msg\"] = \"line\\nbreak\\ttab\\\\slash\\\"quote\\001ctrl\",\n",
            "\t\t\t\t[\"list\"] =\n\t\t\t\t{\n",
            "\t\t\t\t\t[1] = \"first\",\n",
            "\t\t\t\t\t[2] = \"second\",\n",
            "\t\t\t\t\t[3] = 100,\n",
            "\t\t\t\t},\n",
            "\t\t\t\t[\"empty\"] =\n\t\t\t\t{\n\t\t\t\t},\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t},\n}\n",
        );
        let tree = parser::parse_sv_file(input, "complex.lua").unwrap();
        // Inject a high byte value that String::push encodes as two UTF-8 bytes,
        // exercising the escape_lua_string high-byte path in both sinks.
        let mut tree = tree;
        if let Some(top) = tree.children.as_mut().and_then(|c| c.get_mut(0)) {
            top.children.as_mut().unwrap().push(SvTreeNode {
                key: "high\u{00E9}key".into(),
                value_type: SvValueType::String,
                value: Some(serde_json::json!("v\u{00E9}alue")),
                children: None,
                raw_lua_value: None,
            });
        }

        let serialized = serialize_to_lua(&tree);
        assert_eq!(
            serialized_len(&tree),
            serialized.len(),
            "serialized_len must be byte-identical to serialize_to_lua().len()"
        );
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
