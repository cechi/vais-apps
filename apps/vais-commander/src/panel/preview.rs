//! The preview of the entry under a panel's cursor, shown under or beside the
//! list: images (local ones at full size, others as their thumbnail), the
//! start of text files in a read-only editor with syntax highlighting, and
//! an information card for everything else.
//!
//! The panel hands over its cursor entry on every render; reading happens on
//! the background executor after a short pause, so holding an arrow key does
//! not read every file passed.

use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{Icon, IconName, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::FilePanel;
use super::view::Column;
use crate::fs::Entry;
use crate::fs::format::{format_mode, format_time, human_size, owner_name};
use crate::settings::FileIcons;
use crate::ui::{h_flex, v_flex};
use crate::vfs::{self, Location};

/// How much of a text file is shown.
const MAX_TEXT: u64 = 512 * 1024;
/// Bytes checked for NUL to tell binary files from text (as the editor does).
const BINARY_PROBE: usize = 8192;
/// The pause after the cursor moves before a file is read.
const DEBOUNCE: Duration = Duration::from_millis(120);

/// What a target is compared by: a reload that changes the file shows it anew.
type Key = (Location, u64, Option<SystemTime>);

enum Content {
    Empty,
    /// A local image, decoded by GPUI.
    Image(Arc<Path>),
    /// A remote or archived image: its thumbnail, generated on demand.
    Thumbnail,
    /// The start of a text file, in `editor`.
    Text {
        truncated: bool,
    },
    /// The information card; `items` is the entry count of a directory once known.
    Info {
        items: Option<usize>,
    },
    Error(String),
}

/// What the background read found.
enum Loaded {
    Text { text: String, truncated: bool },
    Binary,
    Items(usize),
}

pub struct PreviewPane {
    /// The panel, re-rendered when a thumbnail it asked for is ready.
    panel: WeakEntity<FilePanel>,
    key: Option<Key>,
    entry: Option<Entry>,
    content: Content,
    /// Kept between files: a new editor per file would lose the highlighter
    /// set-up and flicker.
    editor: Option<Entity<EditorState>>,
    _load: Option<Task<()>>,
}

impl PreviewPane {
    pub fn new(panel: WeakEntity<FilePanel>) -> Self {
        Self {
            panel,
            key: None,
            entry: None,
            content: Content::Empty,
            editor: None,
            _load: None,
        }
    }

    /// Show `entry` (the cursor entry; `None` or `..` clear the preview).
    pub fn set_target(
        &mut self,
        entry: Option<&Entry>,
        show_hidden: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry = entry.filter(|e| !e.is_parent);
        let key = entry.map(|e| (e.location.clone(), e.size, e.modified));
        if key == self.key {
            return;
        }
        self.key = key;
        self.entry = entry.cloned();
        self._load = None;
        let Some(entry) = entry.cloned() else {
            self.content = Content::Empty;
            cx.notify();
            return;
        };

        if entry.is_dir() {
            self.content = Content::Info { items: None };
            self.load(entry, show_hidden, window, cx);
        } else if crate::thumbs::supports(&entry.name) {
            self.content = match entry.location.as_local() {
                Some(path) => Content::Image(Arc::from(path)),
                None => Content::Thumbnail,
            };
        } else if entry.size == 0 {
            self.content = Content::Info { items: None };
        } else {
            // The previous file stays until this one is read, so moving the
            // cursor over text files does not flash an empty pane.
            self.load(entry, show_hidden, window, cx);
        }
        cx.notify();
    }

    fn load(
        &mut self,
        entry: Entry,
        show_hidden: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_dir = entry.is_dir();
        let location = entry.location.clone();
        let executor = cx.background_executor().clone();
        self._load = Some(cx.spawn_in(window, async move |this, cx| {
            executor.timer(DEBOUNCE).await;
            let result = executor
                .spawn(async move {
                    if is_dir {
                        read_items(&location, show_hidden)
                    } else {
                        read_text(&location)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.content = match result {
                    Ok(Loaded::Items(n)) => Content::Info { items: Some(n) },
                    Ok(Loaded::Binary) => Content::Info { items: None },
                    Ok(Loaded::Text { text, truncated }) => {
                        this.show_text(&entry.name, text, window, cx);
                        Content::Text { truncated }
                    }
                    Err(err) => Content::Error(err),
                };
                cx.notify();
            });
        }));
    }

    fn show_text(&mut self, name: &str, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let language = crate::editor_view::language_for(name).unwrap_or("text");
        match &self.editor {
            Some(editor) => editor.update(cx, |state, cx| {
                state.set_highlighter(language, cx);
                state.set_value(text, window, cx);
            }),
            None => {
                let editor = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        .line_number(true)
                        .language(language)
                        .default_value(text);
                    state.set_readonly(true, cx);
                    state
                });
                self.editor = Some(editor);
            }
        }
    }

    /// Name, type, size, dates, permissions and owner of the entry.
    fn render_info(&self, entry: &Entry, items: Option<usize>, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let system_icon = (!entry.is_dir()
            && crate::settings::get(cx).file_icons == FileIcons::System)
            .then(|| crate::icons::file_icon(cx, &entry.name))
            .flatten();
        let icon: AnyElement = match system_icon {
            Some(source) => img(source)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None if entry.is_dir() => Icon::new(IconName::Folder)
                .size_full()
                .text_color(theme.blue)
                .into_any_element(),
            None => Icon::new(IconName::File)
                .size_full()
                .text_color(muted)
                .into_any_element(),
        };

        let kind = if entry.is_dir() {
            "<DIR>".to_string()
        } else {
            Path::new(&entry.name)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let size = if entry.is_dir() {
            items.map(|n| tn!("{n} item", "{n} items", n))
        } else {
            Some(human_size(entry.size))
        };
        let rows = [
            (Column::Kind.label(), Some(kind).filter(|k| !k.is_empty())),
            (Column::Size.label(), size),
            (Column::Modified.label(), entry.modified.map(format_time)),
            (Column::Created.label(), entry.created.map(format_time)),
            (
                Column::Permissions.label(),
                entry.mode.map(|m| format_mode(m, &entry.kind)),
            ),
            (Column::Owner.label(), entry.uid.map(owner_name)),
        ];
        let name = if entry.is_symlink() {
            format!("{} →", entry.name)
        } else {
            entry.name.clone()
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_4()
            .gap_3()
            .child(div().size(rems(4.)).flex_none().child(icon))
            .child(div().max_w_full().font_semibold().text_center().child(name))
            .child(
                v_flex()
                    .gap_0p5()
                    .text_sm()
                    .children(rows.into_iter().filter_map(|(label, value)| {
                        let value = value?;
                        Some(
                            h_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .w(rems(7.))
                                        .flex_none()
                                        .text_right()
                                        .text_color(muted)
                                        .child(label),
                                )
                                .child(div().child(value)),
                        )
                    })),
            )
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, muted, danger) = (theme.background, theme.muted_foreground, theme.danger);
        let pane = div()
            .id("preview")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(bg);
        let Some(entry) = self.entry.clone() else {
            return pane;
        };
        let image = |source: ImageSource| {
            div()
                .size_full()
                .p_2()
                .child(img(source).size_full().object_fit(ObjectFit::Contain))
        };
        match &self.content {
            Content::Empty => pane,
            Content::Image(path) => pane.child(image(path.clone().into())),
            Content::Thumbnail => match crate::thumbs::request(cx, &entry, self.panel.clone()) {
                Some(path) => pane.child(image(path.into())),
                None => pane.child(self.render_info(&entry, None, cx)),
            },
            Content::Text { truncated } => {
                let truncated = *truncated;
                let Some(editor) = &self.editor else {
                    return pane;
                };
                pane.child(
                    v_flex()
                        .size_full()
                        .when(truncated, |col| {
                            col.child(
                                div()
                                    .flex_none()
                                    .px_2()
                                    .py_0p5()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!(
                                        "Showing the first {size}",
                                        size = human_size(MAX_TEXT)
                                    )),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(Editor::new(editor).h_full().bordered(false).readonly(true)),
                        ),
                )
            }
            Content::Info { items } => {
                let items = *items;
                pane.child(self.render_info(&entry, items, cx))
            }
            Content::Error(err) => pane.child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .gap_2()
                    .child(div().font_semibold().child(entry.name.clone()))
                    .child(div().text_sm().text_color(danger).child(err.clone())),
            ),
        }
    }
}

/// The start of a text file, or `Binary` when it has NUL bytes early on.
fn read_text(location: &Location) -> Result<Loaded, String> {
    let backend = vfs::backend_for(location).map_err(|e| e.to_string())?;
    // One byte past the limit tells whether the file goes on.
    let mut reader = backend
        .open_read(location)
        .map_err(|e| e.to_string())?
        .take(MAX_TEXT + 1);
    // The probe first, so a remote binary file is not downloaded further.
    let mut bytes = Vec::new();
    (&mut reader)
        .take(BINARY_PROBE as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.contains(&0) {
        return Ok(Loaded::Binary);
    }
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > MAX_TEXT;
    bytes.truncate(MAX_TEXT as usize);
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    Ok(Loaded::Text { text, truncated })
}

fn read_items(location: &Location, show_hidden: bool) -> Result<Loaded, String> {
    let backend = vfs::backend_for(location).map_err(|e| e.to_string())?;
    let entries = backend
        .read_dir(location, show_hidden)
        .map_err(|e| e.to_string())?;
    Ok(Loaded::Items(entries.len()))
}
