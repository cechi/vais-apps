//! What the bridge offers: the tool catalogue (name, description, JSON
//! schema of the arguments) the MCP server advertises, and argument helpers.

use serde_json::{Value, json};

use super::ToolError;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_state",
            description: "What VAIS Commander shows: every tab with its layout and panels (directory, cursor entry, marked entries), which tab and panel are active.",
            input_schema: no_args(),
        },
        ToolSpec {
            name: "list_dir",
            description: "List a directory as the file manager sees it (local path, `conn:/path` on a saved connection, or `archive.zip!/dir` inside an archive).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list" },
                    "show_hidden": { "type": "boolean", "description": "Include dot files (default false)" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "navigate",
            description: "Show a directory in a file panel (the active panel of the active tab unless `tab` / `panel` say otherwise).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to open" },
                    "tab": { "type": "integer", "description": "Tab index (0-based), default the active tab" },
                    "panel": { "type": "integer", "description": "Panel index within the tab (0-based), default the active panel" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
    ]
}

/// The catalogue as MCP's `tools/list` wants it.
pub fn catalog() -> Value {
    Value::Array(
        specs()
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                })
            })
            .collect(),
    )
}

pub fn arg_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError::invalid(format!("`{name}` (string) is required")))
}

pub fn arg_usize(args: &Value, name: &str) -> Result<Option<usize>, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| ToolError::invalid(format!("`{name}` must be a non-negative integer"))),
    }
}

pub fn arg_bool(args: &Value, name: &str, default: bool) -> Result<bool, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| ToolError::invalid(format!("`{name}` must be a boolean"))),
    }
}

#[cfg(test)]
mod tests {
    use super::{catalog, specs};

    #[test]
    fn every_tool_has_an_object_schema() {
        for spec in specs() {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert!(
                spec.input_schema.get("properties").is_some(),
                "{}",
                spec.name
            );
        }
        assert_eq!(catalog().as_array().map(|a| a.len()), Some(specs().len()));
    }
}
