//! The D-Bus side of the picker: ashpd serves
//! `org.freedesktop.impl.portal.FileChooser` (and the `Request` objects) and
//! calls [`VaisChooser`], which hands each request to the GUI thread over a
//! channel and waits for the user's choice.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ashpd::backend::Result;
use ashpd::backend::file_chooser::FileChooserImpl;
use ashpd::backend::request::RequestImpl;
use ashpd::desktop::HandleToken;
use ashpd::desktop::file_chooser::{
    Choice, FileFilter, OpenFileOptions, SaveFileOptions, SaveFilesOptions, SelectedFiles,
};
use ashpd::{MaybeAppID, PortalError, WindowIdentifierType};
use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;

use super::uri::{FilterSet, file_uri, strip_mnemonic};

/// What the caller wants chosen.
#[derive(Clone, Debug)]
pub enum PickKind {
    Open {
        multiple: bool,
        directory: bool,
    },
    Save {
        current_name: Option<String>,
        current_file: Option<PathBuf>,
    },
    /// A directory for these names (the names are kept, made unique on
    /// conflicts).
    SaveFiles {
        files: Vec<String>,
    },
}

/// One request as the picker window sees it.
#[derive(Clone, Debug)]
pub struct PickSpec {
    pub title: String,
    /// The accept button's label, without its mnemonic.
    pub accept_label: Option<String>,
    pub kind: PickKind,
    /// The directory the caller suggests.
    pub start_dir: Option<PathBuf>,
    pub filters: Vec<FilterSet>,
    /// Index into `filters` of the one to start with.
    pub current_filter: Option<usize>,
    /// The caller's combo boxes with their default values; the picker does
    /// not show them yet and reports the defaults back.
    pub choices: Vec<(String, String)>,
    pub app_id: String,
}

impl PickSpec {
    pub fn method(&self) -> &'static str {
        match self.kind {
            PickKind::Open { .. } => "OpenFile",
            PickKind::Save { .. } => "SaveFile",
            PickKind::SaveFiles { .. } => "SaveFiles",
        }
    }
}

/// The user's answer.
#[derive(Debug)]
pub enum PickResult {
    Accepted {
        paths: Vec<PathBuf>,
        /// The filter selected at the end.
        filter: Option<usize>,
    },
    /// Cancel, Escape.
    Cancelled,
    /// The window went away without a choice (closed by the compositor, or
    /// the portal closed the request).
    Closed,
}

/// The answer slot of a request: answering twice does nothing.
#[derive(Clone)]
pub struct Reply(Arc<Mutex<Option<oneshot::Sender<PickResult>>>>);

impl Reply {
    pub fn new(sender: oneshot::Sender<PickResult>) -> Self {
        Self(Arc::new(Mutex::new(Some(sender))))
    }

    pub fn send(&self, result: PickResult) {
        if let Ok(mut slot) = self.0.lock()
            && let Some(sender) = slot.take()
        {
            let _ = sender.send(result);
        }
    }
}

/// From the D-Bus thread to the GUI.
pub enum PortalMsg {
    Pick {
        token: HandleToken,
        spec: PickSpec,
        reply: Reply,
    },
    /// The portal closed the request: drop its window.
    Close(HandleToken),
}

pub struct VaisChooser {
    tx: UnboundedSender<PortalMsg>,
}

impl VaisChooser {
    pub fn new(tx: UnboundedSender<PortalMsg>) -> Self {
        Self { tx }
    }

    /// Hand the request to the GUI and wait for the user.
    async fn ask(&self, token: HandleToken, spec: PickSpec) -> Result<PickResult> {
        log::info!(
            "portal: {} for {:?}: {:?} (start dir {:?})",
            spec.method(),
            spec.app_id,
            spec.title,
            spec.start_dir
        );
        let (tx, rx) = oneshot::channel();
        self.tx
            .unbounded_send(PortalMsg::Pick {
                token,
                spec,
                reply: Reply::new(tx),
            })
            .map_err(|_| PortalError::Failed("the picker is not running".into()))?;
        rx.await
            .map_err(|_| PortalError::Failed("the picker went away".into()))
    }
}

#[ashpd::async_trait::async_trait]
impl RequestImpl for VaisChooser {
    async fn close(&self, token: HandleToken) {
        log::info!("portal: request {token} closed by the portal");
        let _ = self.tx.unbounded_send(PortalMsg::Close(token));
    }
}

#[ashpd::async_trait::async_trait]
impl FileChooserImpl for VaisChooser {
    async fn open_file(
        &self,
        token: HandleToken,
        app_id: Option<MaybeAppID>,
        window_identifier: Option<WindowIdentifierType>,
        title: &str,
        options: OpenFileOptions,
    ) -> Result<SelectedFiles> {
        log_parent(&window_identifier);
        let filters = filter_sets(options.filters());
        let spec = PickSpec {
            title: title.to_string(),
            accept_label: options.accept_label().map(strip_mnemonic),
            kind: PickKind::Open {
                multiple: options.multiple().unwrap_or(false),
                directory: options.directory().unwrap_or(false),
            },
            start_dir: options.current_folder().map(|p| p.as_ref().to_path_buf()),
            current_filter: current_filter_index(options.filters(), options.current_filter()),
            filters,
            choices: default_choices(options.choices()),
            app_id: app_id_text(&app_id),
        };
        let choices = spec.choices.clone();
        let result = self.ask(token, spec).await?;
        selected_files(result, options.filters(), &choices, true)
    }

    async fn save_file(
        &self,
        token: HandleToken,
        app_id: Option<MaybeAppID>,
        window_identifier: Option<WindowIdentifierType>,
        title: &str,
        options: SaveFileOptions,
    ) -> Result<SelectedFiles> {
        log_parent(&window_identifier);
        let filters = filter_sets(options.filters());
        let current_file = options.current_file().map(|p| p.as_ref().to_path_buf());
        let spec = PickSpec {
            title: title.to_string(),
            accept_label: options.accept_label().map(strip_mnemonic),
            kind: PickKind::Save {
                current_name: options.current_name().map(str::to_string),
                current_file: current_file.clone(),
            },
            start_dir: options
                .current_folder()
                .map(|p| p.as_ref().to_path_buf())
                .or_else(|| current_file.and_then(|f| f.parent().map(Path::to_path_buf))),
            current_filter: current_filter_index(options.filters(), options.current_filter()),
            filters,
            choices: default_choices(options.choices()),
            app_id: app_id_text(&app_id),
        };
        let choices = spec.choices.clone();
        let result = self.ask(token, spec).await?;
        selected_files(result, options.filters(), &choices, false)
    }

    async fn save_files(
        &self,
        token: HandleToken,
        app_id: Option<MaybeAppID>,
        window_identifier: Option<WindowIdentifierType>,
        title: &str,
        options: SaveFilesOptions,
    ) -> Result<SelectedFiles> {
        log_parent(&window_identifier);
        let files = options
            .files()
            .iter()
            .filter_map(|f| f.as_ref().file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let spec = PickSpec {
            title: title.to_string(),
            accept_label: options.accept_label().map(strip_mnemonic),
            kind: PickKind::SaveFiles { files },
            start_dir: options.current_folder().map(|p| p.as_ref().to_path_buf()),
            filters: Vec::new(),
            current_filter: None,
            choices: default_choices(options.choices()),
            app_id: app_id_text(&app_id),
        };
        let choices = spec.choices.clone();
        let result = self.ask(token, spec).await?;
        selected_files(result, &[], &choices, false)
    }
}

use std::path::Path;

fn log_parent(window_identifier: &Option<WindowIdentifierType>) {
    // gpui cannot parent a window to a foreign one; the identifier is only
    // logged so a report can tell where the request came from.
    if let Some(id) = window_identifier {
        log::debug!("portal: parent window {id}");
    }
}

fn app_id_text(app_id: &Option<MaybeAppID>) -> String {
    app_id.as_ref().map(|id| id.to_string()).unwrap_or_default()
}

fn filter_sets(filters: &[FileFilter]) -> Vec<FilterSet> {
    filters
        .iter()
        .map(|f| FilterSet {
            label: f.label().to_string(),
            globs: f.pattern_filters().iter().map(|p| p.to_string()).collect(),
            mimes: f.mimetype_filters().iter().map(|m| m.to_string()).collect(),
        })
        .collect()
}

fn current_filter_index(filters: &[FileFilter], current: Option<&FileFilter>) -> Option<usize> {
    let current = current?;
    filters
        .iter()
        .position(|f| f == current)
        .or_else(|| filters.iter().position(|f| f.label() == current.label()))
}

/// The value each combo box reports back: its initial selection, else its
/// first option (a boolean choice without one is off).
fn default_choices(choices: &[Choice]) -> Vec<(String, String)> {
    choices
        .iter()
        .map(|c| {
            let initial = c.initial_selection();
            let value = if !initial.is_empty() {
                initial.to_string()
            } else if let Some((first, _)) = c.pairs().first() {
                first.to_string()
            } else {
                "false".to_string()
            };
            (c.id().to_string(), value)
        })
        .collect()
}

/// The portal's answer for the user's choice.
fn selected_files(
    result: PickResult,
    filters: &[FileFilter],
    choices: &[(String, String)],
    writable: bool,
) -> Result<SelectedFiles> {
    match result {
        PickResult::Accepted { paths, filter } => {
            let mut selected = SelectedFiles::default();
            for path in &paths {
                let uri = ashpd::Uri::parse(&file_uri(path))
                    .map_err(|err| PortalError::Failed(format!("{}: {err:?}", path.display())))?;
                selected = selected.uri(uri);
            }
            if let Some(filter) = filter.and_then(|ix| filters.get(ix)) {
                selected = selected.current_filter(filter.clone());
            }
            if writable {
                selected = selected.writable(true);
            }
            for (id, value) in choices {
                selected = selected.choice(id, value);
            }
            log::info!("portal: chosen {} path(s)", paths.len());
            Ok(selected)
        }
        PickResult::Cancelled => {
            log::info!("portal: cancelled by the user");
            Err(PortalError::Cancelled("cancelled by the user".into()))
        }
        PickResult::Closed => {
            log::info!("portal: picker window closed");
            Err(PortalError::Failed("the picker window was closed".into()))
        }
    }
}
