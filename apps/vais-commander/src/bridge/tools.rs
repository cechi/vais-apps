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
        ToolSpec {
            name: "set_layout",
            description: "Set the grid of a file tab: rows x cols of panels (1..3 each).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "rows": { "type": "integer", "minimum": 1, "maximum": 3 },
                    "cols": { "type": "integer", "minimum": 1, "maximum": 3 },
                    "tab": { "type": "integer", "description": "Tab index, default the active tab" }
                },
                "required": ["rows", "cols"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "new_tab",
            description: "Open a new file tab (a copy of the active tab's layout), optionally showing `path` in its first panel; it becomes the active tab.",
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "activate_tab",
            description: "Switch to tab `index`.",
            input_schema: json!({
                "type": "object",
                "properties": { "index": { "type": "integer" } },
                "required": ["index"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "close_tab",
            description: "Close tab `index` (the last tab cannot be closed; an editor with unsaved changes asks the user).",
            input_schema: json!({
                "type": "object",
                "properties": { "index": { "type": "integer" } },
                "required": ["index"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "select",
            description: "Mark entries in a panel: by exact `names`, or by a `pattern` (glob with * and ?, or a substring); `mode` replace (default), add, or clear (clears every mark).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "names": { "type": "array", "items": { "type": "string" } },
                    "pattern": { "type": "string" },
                    "mode": { "type": "string", "enum": ["replace", "add", "clear"] },
                    "cursor": { "type": "string", "description": "Also put the cursor on this entry" },
                    "tab": { "type": "integer" },
                    "panel": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "copy",
            description: "Copy files or directories into `dest` as the user would with F5. Existing files make the user choose (overwrite / skip / cancel); the call waits for the operation to finish.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sources": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "dest": { "type": "string", "description": "Destination directory" }
                },
                "required": ["sources", "dest"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "move",
            description: "Move files or directories into `dest` as the user would with F6 (same rules as copy).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sources": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "dest": { "type": "string" }
                },
                "required": ["sources", "dest"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "rename",
            description: "Rename one entry (`new_name` is a name, not a path); never overwrites.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "new_name": { "type": "string" }
                },
                "required": ["path", "new_name"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "mkdir",
            description: "Create a directory (parents included).",
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "delete",
            description: "Delete entries: to the trash unless `permanent`. The user confirms in the app; the call waits for the answer and the operation.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "permanent": { "type": "boolean" }
                },
                "required": ["paths"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "search",
            description: "Find entries below `root`: by name (`query` is a glob with * and ? or a substring) or, with `contents`, by text inside files. Up to `limit` results (default 200).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "query": { "type": "string" },
                    "recursive": { "type": "boolean", "description": "Default true" },
                    "contents": { "type": "boolean", "description": "Search file contents instead of names" },
                    "show_hidden": { "type": "boolean" },
                    "limit": { "type": "integer" }
                },
                "required": ["root", "query"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "open_file",
            description: "Open a text file in an editor tab (read-only by default).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "readonly": { "type": "boolean", "description": "Default true" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "add_favorite",
            description: "Bookmark a directory in the sidebar's Favorites.",
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "save_layout",
            description: "Save a file tab (its grid and panel directories) as a named layout favorite in the sidebar.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "tab": { "type": "integer" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "set_tab_title",
            description: "Give a file tab a fixed title (null returns to the directory name).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": ["string", "null"] },
                    "tab": { "type": "integer" }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "set_tab_color",
            description: "Tag a file tab with a colour (`#rrggbb`, or null to remove it).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "color": { "type": ["string", "null"] },
                    "tab": { "type": "integer" }
                },
                "required": ["color"],
                "additionalProperties": false
            }),
        },
    ]
}

pub fn arg_str_list(args: &Value, name: &str) -> Result<Vec<String>, ToolError> {
    let list = args
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::invalid(format!("`{name}` (array of strings) is required")))?;
    let items: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .collect();
    if items.is_empty() {
        return Err(ToolError::invalid(format!("`{name}` must not be empty")));
    }
    Ok(items)
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
