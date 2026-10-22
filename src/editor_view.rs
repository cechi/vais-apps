//! A text file in a tab: gpui-kit's code editor with line numbers, search
//! and syntax highlighting, plus a small toolbar (read-only switch, save,
//! reload) and a status line.
//!
//! Viewing and editing are the same component: F3 opens a file read-only
//! (nothing can be changed by accident), F4 opens it editable, and the
//! toolbar switches between the two at any time. Files that are not valid
//! UTF-8 open read-only so that saving cannot mangle them.

use crate::vfs::{self, Location};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt as _};
use gpui_kit::component::{
    Disableable as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fs::format::human_size;
use crate::keymap::*;
use crate::ui::{h_flex, v_flex};

/// Files above this size open with the default application instead.
pub const MAX_SIZE: u64 = 8 * 1024 * 1024;

pub enum EditorEvent {
    /// The user wants the tab closed.
    Close,
    /// The file was written; panels showing its directory should reload.
    Saved(Location),
}

/// A file read for the editor.
pub struct Loaded {
    pub location: Location,
    pub text: String,
    /// The file uses CRLF line endings (restored on save).
    pub crlf: bool,
    /// The bytes were not valid UTF-8; the text is lossy and read-only.
    pub lossy: bool,
    pub size: u64,
}

/// Read `path` as text. Errors explain why the file cannot be edited
/// (too big, binary, unreadable).
pub fn load(location: &Location) -> Result<Loaded, String> {
    let backend = vfs::backend_for(location).map_err(|e| e.to_string())?;
    let meta = backend.stat(location).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Err("not a regular file".into());
    }
    if meta.size > MAX_SIZE {
        return Err(format!(
            "too large to edit ({}), opening with the default application",
            human_size(meta.size)
        ));
    }
    let bytes = vfs::read_all(location).map_err(|e| e.to_string())?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Err("binary file, opening with the default application".into());
    }
    let (text, lossy) = match String::from_utf8(bytes) {
        Ok(text) => (text, false),
        Err(err) => (String::from_utf8_lossy(err.as_bytes()).into_owned(), true),
    };
    let crlf = text.contains("\r\n");
    let text = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok(Loaded {
        location: location.clone(),
        text,
        crlf,
        lossy,
        size: meta.size,
    })
}

pub struct EditorView {
    location: Location,
    state: Entity<EditorState>,
    readonly: bool,
    dirty: bool,
    crlf: bool,
    lossy: bool,
    size: u64,
    language: Option<&'static str>,
    error: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for EditorView {}

impl EditorView {
    pub fn new(
        loaded: Loaded,
        readonly: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = language_for(&loaded.location.file_name().unwrap_or_default());
        let readonly = readonly || loaded.lossy;
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .line_number(true)
                .searchable(true)
                .default_value(loaded.text.clone());
            if let Some(language) = language {
                state = state.language(language);
            }
            state
        });
        state.update(cx, |state, cx| state.set_readonly(readonly, cx));
        let subscription = cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                if !this.dirty {
                    this.dirty = true;
                }
                cx.notify();
            }
        });
        Self {
            location: loaded.location,
            state,
            readonly,
            dirty: false,
            crlf: loaded.crlf,
            lossy: loaded.lossy,
            size: loaded.size,
            language,
            error: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn location(&self) -> &Location {
        &self.location
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn title(&self) -> String {
        let name = self
            .location
            .file_name()
            .unwrap_or_else(|| self.location.display());
        if self.dirty {
            format!("{name} ●")
        } else {
            name
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Make the file editable (F4 on a file that is already open read-only).
    pub fn set_readonly(&mut self, readonly: bool, cx: &mut Context<Self>) {
        if self.lossy && !readonly {
            self.error = Some("not valid UTF-8, kept read-only".into());
            cx.notify();
            return;
        }
        if self.readonly != readonly {
            self.readonly = readonly;
            self.state
                .update(cx, |state, cx| state.set_readonly(readonly, cx));
            cx.notify();
        }
    }

    /// Write the text back; CRLF files get their line endings back.
    pub fn save(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.readonly {
            return Err("read-only".into());
        }
        let text = self.state.read(cx).value().to_string();
        let text = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        };
        match vfs::write_all(&self.location, text.as_bytes()) {
            Ok(()) => {
                self.dirty = false;
                self.size = text.len() as u64;
                self.error = None;
                cx.emit(EditorEvent::Saved(self.location.clone()));
                cx.notify();
                Ok(())
            }
            Err(err) => {
                self.error = Some(format!("save failed: {err}"));
                cx.notify();
                Err(err.to_string())
            }
        }
    }

    /// Re-read the file from disk, dropping unsaved changes.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match load(&self.location) {
            Ok(loaded) => {
                self.crlf = loaded.crlf;
                self.lossy = loaded.lossy;
                self.size = loaded.size;
                self.dirty = false;
                self.error = None;
                self.state.update(cx, |state, cx| {
                    state.set_value(loaded.text, window, cx);
                });
                if self.lossy {
                    self.set_readonly(true, cx);
                }
            }
            Err(err) => self.error = Some(err),
        }
        cx.notify();
    }

    fn on_save(&mut self, _: &SaveFile, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save(cx);
    }

    fn on_toggle_readonly(&mut self, _: &ToggleReadOnly, _: &mut Window, cx: &mut Context<Self>) {
        self.set_readonly(!self.readonly, cx);
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted, border, title_bar, status_bar, danger) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.title_bar,
            theme.status_bar,
            theme.danger,
        );
        let (position, lines) = {
            let state = self.state.read(cx);
            (state.cursor_position(), state.text().lines_len())
        };
        let name = self.location.file_name().unwrap_or_default();
        let dir = self
            .location
            .parent()
            .map(|p| p.display())
            .unwrap_or_default();
        let mode_label = if self.readonly {
            "Read-only"
        } else {
            "Editable"
        };
        let status = format!(
            "Ln {}, Col {}  ·  {} lines  ·  {}  ·  {}{}{}",
            position.line + 1,
            position.character + 1,
            lines,
            human_size(self.size),
            self.language.unwrap_or("plain text"),
            if self.crlf { "  ·  CRLF" } else { "" },
            if self.lossy { "  ·  not UTF-8" } else { "" },
        );

        v_flex()
            .id("file-editor")
            .key_context(EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_readonly))
            .size_full()
            .min_h_0()
            .bg(bg)
            .text_color(fg)
            .child(
                h_flex()
                    .h(rems(crate::app::HEADER_HEIGHT_REMS))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .bg(title_bar)
                    .border_b_1()
                    .border_color(border)
                    .child(div().font_semibold().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(dir),
                    )
                    .child(
                        Button::new("editor-readonly")
                            .ghost()
                            .xsmall()
                            .icon(if self.readonly {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            })
                            .label(mode_label)
                            .selected(!self.readonly)
                            .disabled(self.lossy)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_readonly(!this.readonly, cx)),
                            ),
                    )
                    .child(
                        Button::new("editor-reload")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .label("Reload")
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
                    .child(
                        Button::new("editor-save")
                            .primary()
                            .xsmall()
                            .icon(IconName::Check)
                            .label("Save")
                            .disabled(self.readonly || !self.dirty)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.save(cx);
                            })),
                    )
                    .child(
                        Button::new("editor-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Close))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // The element pushes its own flag into the state each
                    // render, so it has to carry ours.
                    .child(
                        Editor::new(&self.state)
                            .h_full()
                            .bordered(false)
                            .readonly(self.readonly),
                    ),
            )
            .child(
                h_flex()
                    .h(rems(1.375))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .bg(status_bar)
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(div().flex_1().child(status))
                    .when_some(self.error.clone(), |bar, error| {
                        bar.child(div().text_color(danger).child(error))
                    }),
            )
    }
}

/// The highlighter language for a file, by extension or well-known name.
fn language_for(file_name: &str) -> Option<&'static str> {
    let name = file_name.to_lowercase();
    let by_name = match name.as_str() {
        "makefile" | "gnumakefile" => Some("make"),
        "dockerfile" => Some("dockerfile"),
        "cmakelists.txt" => Some("cmake"),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let ext = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())?
        .1
        .to_string();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" | "scss" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "lua" => "lua",
        "php" => "php",
        "rb" => "ruby",
        "sql" => "sql",
        "swift" => "swift",
        "zig" => "zig",
        "xml" | "svg" | "plist" => "xml",
        "ini" | "cfg" | "conf" => "ini",
        "diff" | "patch" => "diff",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::language_for;

    #[test]
    fn languages_come_from_extensions_and_names() {
        assert_eq!(language_for("main.rs"), Some("rust"));
        assert_eq!(language_for("Makefile"), Some("make"));
        assert_eq!(language_for("notes"), None);
        assert_eq!(language_for("a.unknownext"), None);
        assert_eq!(language_for(".bashrc"), None);
    }
}
