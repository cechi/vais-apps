//! The bridge's tools as the workspace carries them out: a request from
//! another process, answered on the foreground (or from a background task
//! for the slow ones).

use gpui_kit::*;
use serde_json::{Value, json};

use super::Workspace;
use crate::bridge::tools::{arg_bool, arg_str, arg_usize};
use crate::bridge::{BridgeRequest, ToolError, ToolResult};
use crate::fs::{Entry, ReadOptions};
use crate::grid_view::GridView;
use crate::vfs::Location;

impl Workspace {
    pub(crate) fn handle_tool(
        &mut self,
        request: BridgeRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::debug!("bridge: {} {}", request.tool, request.args);
        let BridgeRequest { tool, args, reply } = request;
        let result = match tool.as_str() {
            "get_state" => Ok(self.tool_state(cx)),
            "list_dir" => {
                // Reading may go over the network: answer from a background task.
                match self.tool_list_dir(&args) {
                    Ok((location, show_hidden)) => {
                        cx.background_spawn(async move {
                            reply.send(list_dir(&location, show_hidden));
                        })
                        .detach();
                        return;
                    }
                    Err(err) => Err(err),
                }
            }
            "navigate" => self.tool_navigate(&args, window, cx),
            other => Err(ToolError::not_found(format!("unknown tool `{other}`"))),
        };
        reply.send(result);
    }

    /// The grid of tab `tab` (default the active one).
    fn tool_grid(&self, tab: Option<usize>) -> Result<(usize, Entity<GridView>), ToolError> {
        let ix = tab.unwrap_or(self.active_tab);
        let tab = self
            .tabs
            .get(ix)
            .ok_or_else(|| ToolError::not_found(format!("no tab {ix}")))?;
        let grid = tab
            .grid_view()
            .cloned()
            .ok_or_else(|| ToolError::rejected(format!("tab {ix} is not a file tab")))?;
        Ok((ix, grid))
    }

    fn tool_state(&self, cx: &App) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, tab)| {
                let kind = if tab.grid_view().is_some() {
                    "files"
                } else if tab.editor_view().is_some() {
                    "editor"
                } else if tab.settings_view().is_some() {
                    "settings"
                } else if tab.rename_view().is_some() {
                    "rename"
                } else {
                    "connections"
                };
                let mut value = json!({
                    "index": ix,
                    "kind": kind,
                    "title": tab.title(cx),
                    "active": ix == self.active_tab,
                });
                if let Some(grid) = tab.grid_view() {
                    let grid = grid.read(cx);
                    let layout = grid.layout();
                    let shown = usize::from(layout.rows) * usize::from(layout.cols);
                    let panels: Vec<Value> = grid
                        .panels()
                        .iter()
                        .take(shown)
                        .enumerate()
                        .map(|(pix, panel)| {
                            let panel = panel.read(cx);
                            let state = &panel.state;
                            json!({
                                "index": pix,
                                "active": pix == grid.active_index(),
                                "path": state.path.display(),
                                "cursor": state.current().filter(|e| !e.is_parent).map(|e| e.name.clone()),
                                "marked": state.selected_marked_names(),
                                "show_hidden": state.show_hidden,
                                "sort": serde_json::to_value(state.sort.key).unwrap_or(Value::Null),
                                "filter": state.filter.as_ref().map(|f| json!({
                                    "query": f.query, "subfolders": f.recursive, "contents": f.content
                                })),
                                "error": state.error,
                            })
                        })
                        .collect();
                    value["color"] = json!(grid.color());
                    value["layout"] = json!({ "rows": layout.rows, "cols": layout.cols });
                    value["active_panel"] = json!(grid.active_index());
                    value["panels"] = Value::Array(panels);
                }
                value
            })
            .collect();
        json!({
            "active_tab": self.active_tab,
            "tabs": tabs,
            "agents_open": self.agents_open,
        })
    }

    fn tool_list_dir(&self, args: &Value) -> Result<(Location, bool), ToolError> {
        let path = arg_str(args, "path")?;
        let show_hidden = arg_bool(args, "show_hidden", false)?;
        Ok((crate::panel::expand_path(path.trim()), show_hidden))
    }

    fn tool_navigate(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let path = crate::panel::expand_path(arg_str(args, "path")?.trim());
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        let panel_ix = arg_usize(args, "panel")?;
        let panel = {
            let grid = grid.read(cx);
            match panel_ix {
                Some(ix) => grid.panels().get(ix).cloned().ok_or_else(|| {
                    ToolError::not_found(format!("no panel {ix} in tab {tab_ix}"))
                })?,
                None => grid.active_panel().clone(),
            }
        };
        let _ = window;
        panel.update(cx, |panel, cx| panel.navigate_to(path.clone(), cx));
        Ok(
            json!({ "tab": tab_ix, "panel": panel_ix.unwrap_or_else(|| grid.read(cx).active_index()), "path": path.display() }),
        )
    }
}

/// The listing of `location` as JSON (runs on a background thread).
fn list_dir(location: &Location, show_hidden: bool) -> ToolResult {
    let opts = ReadOptions {
        show_hidden,
        ..ReadOptions::default()
    };
    let entries = crate::fs::read_dir_sorted(location, opts)
        .map_err(|e| ToolError::new("not_found", format!("{}: {e}", location.display())))?;
    let items: Vec<Value> = entries
        .iter()
        .filter(|e| !e.is_parent)
        .map(entry_json)
        .collect();
    Ok(json!({ "path": location.display(), "entries": items }))
}

fn entry_json(entry: &Entry) -> Value {
    let kind = if entry.is_dir() {
        "dir"
    } else if entry.is_symlink() {
        "symlink"
    } else {
        "file"
    };
    json!({
        "name": entry.name,
        "path": entry.location.display(),
        "kind": kind,
        "size": entry.size,
        "modified": entry.modified.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()),
    })
}
