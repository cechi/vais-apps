//! The bridge's tools as the workspace carries them out: a request from
//! another process, answered on the foreground (or from a background task
//! for the slow ones).

use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use gpui_kit::*;
use serde_json::{Value, json};

use super::Workspace;
use crate::bridge::tools::{arg_bool, arg_str, arg_str_list, arg_usize};
use crate::bridge::{BridgeRequest, Reply, ToolError, ToolResult};
use crate::config::FavoriteLayout;
use crate::fs::search::{Needle, search_tree};
use crate::fs::{Entry, ReadOptions};
use crate::grid_view::{GridView, OnOutcome, Outcome};
use crate::layout::Layout;
use crate::ops::OpKind;
use crate::ops::manager::Operation;
use crate::vfs::Location;

/// Results per search before the walk stops.
const SEARCH_DEFAULT_LIMIT: usize = 200;
const SEARCH_MAX_LIMIT: usize = 5000;

impl Workspace {
    pub(crate) fn handle_tool(
        &mut self,
        request: BridgeRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::debug!("bridge: {} {}", request.tool, request.args);
        let BridgeRequest { tool, args, reply } = request;
        // A tool that moves the focus (layout, tabs) must not steal it from
        // the agent's terminal.
        let agents_focus = self.agents.read(cx).focus_handle(cx);
        let had_focus = self.agents_open && agents_focus.contains_focused(window, cx);
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
            "search" => match tool_search_args(&args) {
                Ok(search) => {
                    cx.background_spawn(async move {
                        reply.send(run_search(search));
                    })
                    .detach();
                    return;
                }
                Err(err) => Err(err),
            },
            "navigate" => self.tool_navigate(&args, window, cx),
            "set_layout" => self.tool_set_layout(&args, window, cx),
            "new_tab" => self.tool_new_tab(&args, window, cx),
            "activate_tab" => self.tool_activate_tab(&args, window, cx),
            "close_tab" => self.tool_close_tab(&args, window, cx),
            "select" => self.tool_select(&args, cx),
            "copy" => match self.tool_transfer(OpKind::Copy, &args, reply.clone(), window, cx) {
                Ok(()) => return,
                Err(err) => Err(err),
            },
            "move" => match self.tool_transfer(OpKind::Move, &args, reply.clone(), window, cx) {
                Ok(()) => return,
                Err(err) => Err(err),
            },
            "delete" => match self.tool_delete(&args, reply.clone(), window, cx) {
                Ok(()) => return,
                Err(err) => Err(err),
            },
            "rename" => self.tool_rename(&args, cx),
            "mkdir" => self.tool_mkdir(&args, cx),
            "open_file" => self.tool_open_file(&args, window, cx),
            "add_favorite" => self.tool_add_favorite(&args, window, cx),
            "save_layout" => self.tool_save_layout(&args, cx),
            "set_tab_title" => self.tool_set_tab_title(&args, cx),
            "set_tab_color" => self.tool_set_tab_color(&args, cx),
            other => Err(ToolError::not_found(format!("unknown tool `{other}`"))),
        };
        if had_focus && !agents_focus.contains_focused(window, cx) {
            agents_focus.focus(window, cx);
        }
        reply.send(result);
    }

    /// An operation the bridge waits for ended.
    pub(crate) fn tool_operation_finished(&mut self, op: &Operation) {
        if let Some(reply) = self.bridge_waits.remove(&op.id) {
            reply.send(Ok(operation_json(op)));
        }
    }

    /// A grid for a tool that names its paths itself: the active tab when
    /// it shows files, else the first file tab (an editor may be in front).
    fn tool_any_grid(&self) -> Result<Entity<GridView>, ToolError> {
        if let Ok((_, grid)) = self.tool_grid(None) {
            return Ok(grid);
        }
        self.tabs
            .iter()
            .find_map(|t| t.grid_view().cloned())
            .ok_or_else(|| ToolError::rejected("no file tab is open"))
    }

    /// The panel `panel` (default the active one) of the grid.
    fn tool_panel(
        &self,
        grid: &Entity<GridView>,
        tab_ix: usize,
        panel: Option<usize>,
        cx: &App,
    ) -> Result<(usize, Entity<crate::panel::FilePanel>), ToolError> {
        let grid = grid.read(cx);
        match panel {
            Some(ix) => {
                let shown = grid.layout().count();
                grid.panels()
                    .get(ix)
                    .filter(|_| ix < shown)
                    .cloned()
                    .map(|p| (ix, p))
                    .ok_or_else(|| ToolError::not_found(format!("no panel {ix} in tab {tab_ix}")))
            }
            None => Ok((grid.active_index(), grid.active_panel().clone())),
        }
    }

    fn tool_set_layout(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let rows =
            arg_usize(args, "rows")?.ok_or_else(|| ToolError::invalid("`rows` is required"))?;
        let cols =
            arg_usize(args, "cols")?.ok_or_else(|| ToolError::invalid("`cols` is required"))?;
        if !(1..=3).contains(&rows) || !(1..=3).contains(&cols) {
            return Err(ToolError::invalid("rows and cols must be between 1 and 3"));
        }
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        grid.update(cx, |grid, cx| {
            grid.set_layout(Layout::new(rows as u8, cols as u8), window, cx)
        });
        self.schedule_save(cx);
        Ok(json!({ "tab": tab_ix, "rows": rows, "cols": cols }))
    }

    fn tool_new_tab(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        self.new_tab(window, cx);
        let ix = self.active_tab;
        if let Some(path) = args
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.trim().is_empty())
        {
            let location = crate::panel::expand_path(path.trim());
            if let Some(grid) = self.tabs[ix].grid_view() {
                let panel = grid.read(cx).active_panel().clone();
                panel.update(cx, |panel, cx| panel.navigate_to(location, cx));
            }
        }
        Ok(json!({ "index": ix }))
    }

    fn tool_activate_tab(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let ix =
            arg_usize(args, "index")?.ok_or_else(|| ToolError::invalid("`index` is required"))?;
        if ix >= self.tabs.len() {
            return Err(ToolError::not_found(format!("no tab {ix}")));
        }
        self.activate_tab(ix, window, cx);
        Ok(json!({ "index": ix, "title": self.tabs[ix].title(cx) }))
    }

    fn tool_close_tab(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let ix =
            arg_usize(args, "index")?.ok_or_else(|| ToolError::invalid("`index` is required"))?;
        if ix >= self.tabs.len() {
            return Err(ToolError::not_found(format!("no tab {ix}")));
        }
        if self.tabs.len() <= 1 {
            return Err(ToolError::rejected("the last tab stays open"));
        }
        let title = self.tabs[ix].title(cx);
        self.close_tab(ix, window, cx);
        Ok(json!({ "closed": title, "tabs": self.tabs.len() }))
    }

    fn tool_select(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        let (panel_ix, panel) = self.tool_panel(&grid, tab_ix, arg_usize(args, "panel")?, cx)?;
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("replace");
        let add = match mode {
            "replace" => false,
            "add" => true,
            "clear" => {
                panel.update(cx, |panel, cx| panel.clear_marks(cx));
                return Ok(json!({ "tab": tab_ix, "panel": panel_ix, "marked": 0 }));
            }
            other => return Err(ToolError::invalid(format!("unknown mode `{other}`"))),
        };
        let matched = if let Some(names) = args.get("names").and_then(Value::as_array) {
            let names: Vec<String> = names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            panel.update(cx, |panel, cx| panel.select_names(&names, add, cx))
        } else if let Some(pattern) = args.get("pattern").and_then(Value::as_str) {
            panel.update(cx, |panel, cx| {
                if !add {
                    panel.clear_marks(cx);
                }
                panel.mark_pattern(pattern, true, cx);
                panel.state.marked.len()
            })
        } else {
            return Err(ToolError::invalid("`names` or `pattern` is required"));
        };
        let cursor = match args.get("cursor").and_then(Value::as_str) {
            Some(name) => Some(panel.update(cx, |panel, cx| panel.set_cursor_to(name, cx))),
            None => None,
        };
        let marked = panel.read(cx).state.selected_marked_names();
        Ok(
            json!({ "tab": tab_ix, "panel": panel_ix, "matched": matched, "marked": marked, "cursor_found": cursor }),
        )
    }

    /// Copy / move through the grid's own confirmation flow; the reply is
    /// stored until the operation finishes (or the user cancels).
    fn tool_transfer(
        &mut self,
        kind: OpKind,
        args: &Value,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ToolError> {
        let sources: Vec<Location> = arg_str_list(args, "sources")?
            .iter()
            .map(|p| crate::panel::expand_path(p.trim()))
            .collect();
        let dest = crate::panel::expand_path(arg_str(args, "dest")?.trim());
        let grid = self.tool_any_grid()?;
        let on_outcome = self.tool_outcome(reply, cx);
        grid.update(cx, |grid, cx| {
            grid.confirm_transfer_with(kind, sources, dest, Some(on_outcome), window, cx)
        });
        Ok(())
    }

    fn tool_delete(
        &mut self,
        args: &Value,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ToolError> {
        let paths: Vec<Location> = arg_str_list(args, "paths")?
            .iter()
            .map(|p| crate::panel::expand_path(p.trim()))
            .collect();
        if paths.iter().any(Location::is_roots) {
            return Err(ToolError::rejected("not the roots listing"));
        }
        let permanent = arg_bool(args, "permanent", false)?;
        let grid = self.tool_any_grid()?;
        let on_outcome = self.tool_outcome(reply, cx);
        grid.update(cx, |grid, cx| {
            grid.confirm_delete(
                paths,
                permanent,
                Some("The agent".into()),
                Some(on_outcome),
                window,
                cx,
            )
        });
        Ok(())
    }

    /// The callback a confirmation flow reports to: a started operation
    /// parks the reply until it finishes, anything else answers right away.
    fn tool_outcome(&self, reply: Reply, cx: &mut Context<Self>) -> OnOutcome {
        let workspace = cx.entity();
        Rc::new(
            move |outcome: Outcome, window: &mut Window, cx: &mut App| match outcome {
                Outcome::Started(id) => {
                    // The workspace may be the caller (still borrowed): park the
                    // reply once this update is over.
                    let (reply, workspace) = (reply.clone(), workspace.clone());
                    window.defer(cx, move |_, cx| {
                        workspace.update(cx, |this, _| {
                            this.bridge_waits.insert(id, reply);
                        });
                    });
                }
                Outcome::Cancelled => {
                    reply.send(Err(ToolError::new("cancelled", "the user cancelled")))
                }
                Outcome::Rejected(message) => reply.send(Err(ToolError::rejected(message))),
            },
        )
    }

    fn tool_rename(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let path = crate::panel::expand_path(arg_str(args, "path")?.trim());
        let new_name = arg_str(args, "new_name")?.trim().to_string();
        if new_name.contains(['/', '\\']) {
            return Err(ToolError::invalid("`new_name` is a name, not a path"));
        }
        let base = path
            .parent()
            .ok_or_else(|| ToolError::rejected("cannot rename a root"))?;
        let old_name = path
            .file_name()
            .ok_or_else(|| ToolError::invalid("no file name"))?;
        let grid = self.tool_any_grid()?;
        let renamed = grid
            .update(cx, |grid, cx| {
                grid.rename_quiet(&base, &old_name, &new_name, cx)
            })
            .map_err(ToolError::rejected)?;
        Ok(json!({ "path": renamed.display() }))
    }

    fn tool_mkdir(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let path = crate::panel::expand_path(arg_str(args, "path")?.trim());
        let base = path
            .parent()
            .ok_or_else(|| ToolError::rejected("cannot create a root"))?;
        let name = path
            .file_name()
            .ok_or_else(|| ToolError::invalid("no directory name"))?;
        let grid = self.tool_any_grid()?;
        let created = grid
            .update(cx, |grid, cx| grid.make_dir_quiet(&base, &name, cx))
            .map_err(ToolError::rejected)?;
        Ok(json!({ "path": created.display() }))
    }

    fn tool_open_file(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let path = crate::panel::expand_path(arg_str(args, "path")?.trim());
        let readonly = arg_bool(args, "readonly", true)?;
        self.open_file(path.clone(), readonly, window, cx);
        Ok(json!({ "path": path.display(), "readonly": readonly }))
    }

    fn tool_add_favorite(
        &mut self,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolResult {
        let path = crate::panel::expand_path(arg_str(args, "path")?.trim());
        if self.favorites.iter().any(|f| f.path == path) {
            return Ok(json!({ "path": path.display(), "added": false }));
        }
        self.toggle_favorite(path.clone(), window, cx);
        Ok(json!({ "path": path.display(), "added": true }))
    }

    fn tool_save_layout(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let name = arg_str(args, "name")?.trim().to_string();
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        let tab = grid.read(cx).config(cx);
        self.favorite_layouts.push(FavoriteLayout {
            name: name.clone(),
            tab,
        });
        self.sync_layouts(cx);
        self.schedule_save(cx);
        Ok(json!({ "name": name, "tab": tab_ix, "layouts": self.favorite_layouts.len() }))
    }

    fn tool_set_tab_title(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        let title = match args.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            Some(_) => return Err(ToolError::invalid("`title` must be a string or null")),
        };
        grid.update(cx, |grid, cx| grid.set_title(title.clone(), cx));
        Ok(json!({ "tab": tab_ix, "title": self.tabs[tab_ix].title(cx) }))
    }

    fn tool_set_tab_color(&mut self, args: &Value, cx: &mut Context<Self>) -> ToolResult {
        let (tab_ix, grid) = self.tool_grid(arg_usize(args, "tab")?)?;
        let color = match args.get("color") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(
                crate::tab_color::normalise(s)
                    .ok_or_else(|| ToolError::invalid(format!("`{s}` is not a hex colour")))?,
            ),
            Some(_) => return Err(ToolError::invalid("`color` must be a string or null")),
        };
        grid.update(cx, |grid, cx| grid.set_color(color.clone(), cx));
        Ok(json!({ "tab": tab_ix, "color": color }))
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

struct SearchArgs {
    root: Location,
    needle: Needle,
    recursive: bool,
    show_hidden: bool,
    limit: usize,
}

fn tool_search_args(args: &Value) -> Result<SearchArgs, ToolError> {
    let root = crate::panel::expand_path(arg_str(args, "root")?.trim());
    let query = arg_str(args, "query")?.to_string();
    let contents = arg_bool(args, "contents", false)?;
    let needle = if contents {
        Needle::content(&query)
    } else {
        Needle::Name(crate::fs::filter::Matcher::new(&query))
    };
    Ok(SearchArgs {
        root,
        needle,
        recursive: arg_bool(args, "recursive", true)?,
        show_hidden: arg_bool(args, "show_hidden", false)?,
        limit: arg_usize(args, "limit")?
            .unwrap_or(SEARCH_DEFAULT_LIMIT)
            .clamp(1, SEARCH_MAX_LIMIT),
    })
}

/// The search as JSON (runs on a background thread).
fn run_search(search: SearchArgs) -> ToolResult {
    let opts = ReadOptions {
        show_hidden: search.show_hidden,
        ..ReadOptions::default()
    };
    let cancel = AtomicBool::new(false);
    let (entries, truncated) = search_tree(
        &search.root,
        opts,
        &search.needle,
        search.recursive,
        search.limit,
        &cancel,
    );
    let items: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut v = entry_json(e);
            v["name"] = json!(e.name);
            v
        })
        .collect();
    Ok(json!({ "root": search.root.display(), "entries": items, "truncated": truncated }))
}

fn operation_json(op: &Operation) -> Value {
    let report = op.report.as_ref();
    json!({
        "operation_id": op.id,
        "kind": op.kind.verb(),
        "status": format!("{:?}", op.status),
        "done": report.map(|r| r.done),
        "errors": report.map(|r| r.errors.clone()).unwrap_or_default(),
        "cancelled": report.is_some_and(|r| r.cancelled),
    })
}
