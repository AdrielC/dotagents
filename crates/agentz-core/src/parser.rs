//! **Frontmatter-aware parsers** for the two authoritative per-agent artefact formats we target.
//!
//! - [`cursor::parse_mdc`] reads a Cursor `.mdc` rule: `---\n<yaml>\n---\n<body>`, with the
//!   documented keys `description`, `globs`, `alwaysApply`, plus a loose bag of extras.
//! - [`claude::parse_skill_md`] reads a Claude `SKILL.md`: same shape, different key set
//!   (`name`, `description`, `allowed-tools`, …).
//!
//! Both parsers are *byte-equivalent* round-trippers: parse then re-emit produces the same bytes
//! provided the caller feeds canonical input (trailing newline, unix line endings). We use
//! `serde_json` as an intermediate representation for the frontmatter since every documented key
//! serialises cleanly to JSON and we already depend on `serde_json`. A full YAML parser stays out
//! of the dependency graph.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use thiserror::Error;

const FENCE: &str = "---";

/// Anything that can go wrong while parsing a frontmatter document.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("input is empty")]
    Empty,
    #[error("missing opening `---` fence on line 1 (got: {0:?})")]
    MissingOpenFence(String),
    #[error("missing closing `---` fence")]
    MissingCloseFence,
    #[error("frontmatter: unsupported key `{0}` = `{1}` (expected a scalar, list, or bool)")]
    UnsupportedValue(String, String),
    #[error("required key `{0}` missing from frontmatter")]
    MissingRequired(&'static str),
}

/// Split a frontmatter document into (frontmatter_text, body_text). Neither side includes the
/// fences. The closing fence *must* be followed by a newline (or EOF) so we never accidentally
/// slice in the middle of a body line that happens to start with `---`.
pub fn split_frontmatter(text: &str) -> Result<(&str, &str), ParseError> {
    if text.is_empty() {
        return Err(ParseError::Empty);
    }
    let first_line_end = text.find('\n').unwrap_or(text.len());
    let first_line = text[..first_line_end].trim_end_matches('\r');
    if first_line != FENCE {
        return Err(ParseError::MissingOpenFence(first_line.to_string()));
    }
    let rest = &text[first_line_end + 1..];

    // Find the next line that is exactly `---` (after trimming \r).
    let mut cursor = 0;
    loop {
        let line_end = rest[cursor..]
            .find('\n')
            .map(|n| cursor + n)
            .unwrap_or(rest.len());
        let line = rest[cursor..line_end].trim_end_matches('\r');
        if line == FENCE {
            let fm = &rest[..cursor];
            let body_start = if line_end < rest.len() {
                line_end + 1
            } else {
                rest.len()
            };
            let body = &rest[body_start..];
            return Ok((fm, body));
        }
        if line_end == rest.len() {
            return Err(ParseError::MissingCloseFence);
        }
        cursor = line_end + 1;
    }
}

/// Parse a *very* minimal subset of YAML — flat `key: value` pairs, plus list values either as
/// inline `[a, b, c]` or as a followup set of `  - item` lines. Good enough for Cursor/Claude
/// frontmatter, explicitly rejects anything more exotic so we fail loudly instead of guessing.
///
/// Returned `BTreeMap` values are `serde_json::Value`s: strings for scalars, arrays for lists,
/// booleans for `true`/`false`. Deterministic order.
pub fn parse_simple_yaml(text: &str) -> Result<BTreeMap<String, Value>, ParseError> {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let mut current_list: Option<(String, Vec<Value>)> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');

        // Drop pure-comment / empty lines.
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Indented list item continues the active key.
        if line.starts_with("  - ") || line.starts_with("- ") {
            let item = line.trim_start_matches(' ').trim_start_matches("- ").trim();
            let Some((_, items)) = current_list.as_mut() else {
                return Err(ParseError::UnsupportedValue(
                    "(list continuation)".into(),
                    item.to_string(),
                ));
            };
            items.push(Value::String(strip_surrounding_quotes(item).to_string()));
            continue;
        }

        if let Some((k, items)) = current_list.take() {
            out.insert(k, Value::Array(items));
        }

        let Some((key, value)) = line.split_once(':') else {
            return Err(ParseError::UnsupportedValue(
                "(unparseable line)".into(),
                line.to_string(),
            ));
        };
        let key = key.trim().to_string();
        let value = value.trim();

        if value.is_empty() {
            current_list = Some((key, Vec::new()));
            continue;
        }

        out.insert(key, scalar(value)?);
    }

    if let Some((k, items)) = current_list {
        out.insert(k, Value::Array(items));
    }

    Ok(out)
}

fn scalar(raw: &str) -> Result<Value, ParseError> {
    let trimmed = strip_surrounding_quotes(raw);
    if let Some(rest) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let items: Vec<Value> = rest
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(strip_surrounding_quotes(s).to_string()))
            .collect();
        return Ok(Value::Array(items));
    }
    match trimmed {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        other => Ok(Value::String(other.to_string())),
    }
}

fn strip_surrounding_quotes(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Emit a canonical YAML frontmatter block from a sorted key→value map. The emitter is the exact
/// inverse of [`parse_simple_yaml`]: scalars are quoted when they contain special characters,
/// lists are emitted inline when short and block when long.
#[must_use]
pub fn emit_simple_yaml(fields: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    for (k, v) in fields {
        out.push_str(k);
        out.push_str(": ");
        match v {
            Value::String(s) => {
                out.push_str(&yaml_scalar(s));
            }
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::Array(items) => {
                let all_scalars = items
                    .iter()
                    .all(|i| matches!(i, Value::String(_) | Value::Bool(_) | Value::Number(_)));
                let short = all_scalars && items.len() <= 4;
                if short {
                    out.push('[');
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        match item {
                            Value::String(s) => out.push_str(&yaml_scalar(s)),
                            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                            Value::Number(n) => out.push_str(&n.to_string()),
                            _ => out.push_str("null"),
                        }
                    }
                    out.push(']');
                } else {
                    for item in items {
                        out.push_str("\n  - ");
                        match item {
                            Value::String(s) => out.push_str(&yaml_scalar(s)),
                            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                            _ => out.push_str("null"),
                        }
                    }
                }
            }
            Value::Null => out.push_str("null"),
            Value::Object(_) => out.push_str("null"),
        }
        out.push('\n');
    }
    out
}

fn yaml_scalar(s: &str) -> String {
    if s.contains(':')
        || s.contains('#')
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.contains(',')
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Cursor-specific
// ---------------------------------------------------------------------------

pub mod cursor {
    use super::*;

    /// Parsed Cursor `.mdc` rule. Frontmatter keys preserved verbatim in `extras` if we don't
    /// recognise them.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CursorMdc {
        pub description: Option<String>,
        pub globs: Vec<String>,
        pub always_apply: Option<bool>,
        pub extras: BTreeMap<String, Value>,
        pub body: String,
    }

    /// Parse a Cursor `.mdc` rule file.
    pub fn parse_mdc(text: &str) -> Result<CursorMdc, ParseError> {
        let (fm, body) = split_frontmatter(text)?;
        let mut fields = parse_simple_yaml(fm)?;

        let description = fields.remove("description").and_then(value_as_string);
        let globs = fields
            .remove("globs")
            .map(value_as_string_list)
            .unwrap_or_default();
        let always_apply = fields.remove("alwaysApply").and_then(|v| match v {
            Value::Bool(b) => Some(b),
            Value::String(s) => match s.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        });

        Ok(CursorMdc {
            description,
            globs,
            always_apply,
            extras: fields,
            body: body.to_string(),
        })
    }

    /// Re-emit a [`CursorMdc`] to a byte-for-byte canonical form.
    #[must_use]
    pub fn emit_mdc(m: &CursorMdc) -> String {
        let mut fields: BTreeMap<String, Value> = BTreeMap::new();
        if let Some(d) = &m.description {
            fields.insert("description".into(), Value::String(d.clone()));
        }
        if !m.globs.is_empty() {
            fields.insert(
                "globs".into(),
                Value::Array(m.globs.iter().cloned().map(Value::String).collect()),
            );
        }
        if let Some(a) = m.always_apply {
            fields.insert("alwaysApply".into(), Value::Bool(a));
        }
        for (k, v) in &m.extras {
            fields.insert(k.clone(), v.clone());
        }
        let fm = emit_simple_yaml(&fields);
        format!("{FENCE}\n{fm}{FENCE}\n{}", m.body)
    }
}

// ---------------------------------------------------------------------------
// Claude-specific
// ---------------------------------------------------------------------------

pub mod claude {
    use super::*;

    /// Parsed Claude `SKILL.md`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ClaudeSkill {
        pub name: String,
        pub description: Option<String>,
        pub allowed_tools: Vec<String>,
        pub extras: BTreeMap<String, Value>,
        pub body: String,
    }

    /// Parse a Claude `SKILL.md` file. `name` is required.
    pub fn parse_skill_md(text: &str) -> Result<ClaudeSkill, ParseError> {
        let (fm, body) = split_frontmatter(text)?;
        let mut fields = parse_simple_yaml(fm)?;

        let name = fields
            .remove("name")
            .and_then(value_as_string)
            .ok_or(ParseError::MissingRequired("name"))?;
        let description = fields.remove("description").and_then(value_as_string);
        // Claude uses `allowed-tools` (hyphenated) in SKILL.md.
        let allowed_tools = fields
            .remove("allowed-tools")
            .map(value_as_string_list)
            .unwrap_or_default();

        Ok(ClaudeSkill {
            name,
            description,
            allowed_tools,
            extras: fields,
            body: body.to_string(),
        })
    }

    /// Re-emit a [`ClaudeSkill`] to a canonical `SKILL.md`.
    #[must_use]
    pub fn emit_skill_md(s: &ClaudeSkill) -> String {
        let mut fields: BTreeMap<String, Value> = BTreeMap::new();
        fields.insert("name".into(), Value::String(s.name.clone()));
        if let Some(d) = &s.description {
            fields.insert("description".into(), Value::String(d.clone()));
        }
        if !s.allowed_tools.is_empty() {
            fields.insert(
                "allowed-tools".into(),
                Value::Array(s.allowed_tools.iter().cloned().map(Value::String).collect()),
            );
        }
        for (k, v) in &s.extras {
            fields.insert(k.clone(), v.clone());
        }
        let fm = emit_simple_yaml(&fields);
        format!("{FENCE}\n{fm}{FENCE}\n{}", s.body)
    }
}

fn value_as_string(v: Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_as_string_list(v: Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.into_iter().filter_map(value_as_string).collect(),
        Value::String(s) => vec![s],
        _ => Vec::new(),
    }
}

// Silence a rustc warning: `Map` is unused when serde_json re-exports it, but we keep the import
// available for downstream crates that want to round-trip through `Map<String, Value>`.
#[allow(dead_code)]
type _KeepMapUsed = Map<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter_and_body() {
        let input = "---\nkey: value\n---\nbody line 1\nbody line 2\n";
        let (fm, body) = split_frontmatter(input).unwrap();
        assert_eq!(fm, "key: value\n");
        assert_eq!(body, "body line 1\nbody line 2\n");
    }

    #[test]
    fn missing_open_fence_errors() {
        let err = split_frontmatter("nope\n---\n").unwrap_err();
        assert!(matches!(err, ParseError::MissingOpenFence(_)));
    }

    #[test]
    fn cursor_mdc_round_trip() {
        let input = "---\nalwaysApply: true\ndescription: Coding style\nglobs: [\"**/*.rs\", \"**/*.ts\"]\n---\n# style\n";
        let parsed = cursor::parse_mdc(input).unwrap();
        assert_eq!(parsed.description.as_deref(), Some("Coding style"));
        assert_eq!(parsed.globs, vec!["**/*.rs", "**/*.ts"]);
        assert_eq!(parsed.always_apply, Some(true));
        assert_eq!(parsed.body, "# style\n");

        let reemitted = cursor::emit_mdc(&parsed);
        let reparsed = cursor::parse_mdc(&reemitted).unwrap();
        assert_eq!(parsed, reparsed, "round-trip preserves semantic content");
    }

    #[test]
    fn claude_skill_round_trip() {
        let input = "---\nname: code-reviewer\ndescription: Reviews diffs for regressions\nallowed-tools: [Read, Grep]\n---\nBody goes here.\n";
        let parsed = claude::parse_skill_md(input).unwrap();
        assert_eq!(parsed.name, "code-reviewer");
        assert_eq!(parsed.allowed_tools, vec!["Read", "Grep"]);
        let reemitted = claude::emit_skill_md(&parsed);
        let reparsed = claude::parse_skill_md(&reemitted).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn claude_skill_missing_name_errors() {
        let input = "---\ndescription: nope\n---\nbody";
        let err = claude::parse_skill_md(input).unwrap_err();
        assert!(matches!(err, ParseError::MissingRequired("name")));
    }
}
