//! `vais-commander --portal`: the file chooser other applications open
//! through xdg-desktop-portal. A separate process, started by D-Bus
//! activation, owns `org.freedesktop.impl.portal.desktop.vais`; it has no
//! main window and opens a picker window for each request. It exits once
//! it has been idle for a while (D-Bus starts it again on the next request),
//! when another instance takes the bus name, or when the bus goes away.

mod service;
mod uri;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use ashpd::backend::Builder;
use ashpd::zbus::fdo::RequestNameFlags;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::task::{FutureObj, Spawn, SpawnError};
use gpui_kit::*;

use crate::config::Config;
pub use service::{PickKind, PickResult, PickSpec, PortalMsg, Reply};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.vais";
/// How long the process stays around after its last window closed
/// (`VAIS_PORTAL_IDLE_SECS` overrides it).
const DEFAULT_IDLE: Duration = Duration::from_secs(600);

struct State {
    #[allow(dead_code)] // the picker windows (next step) read it
    config: Config,
    /// The open picker windows by request.
    windows: HashMap<ashpd::desktop::HandleToken, AnyWindowHandle>,
    /// The pending idle exit, while no window is open.
    idle: Option<Task<()>>,
}

/// The whole `--portal` run; returns the exit code.
pub fn run(config: Config) -> i32 {
    let (tx, mut rx) = mpsc::unbounded::<PortalMsg>();
    std::thread::Builder::new()
        .name("portal-dbus".into())
        .spawn(move || {
            match futures::executor::block_on(serve(tx)) {
                Ok(()) => log::info!("portal: bus connection closed"),
                // Usually another instance owns the name already.
                Err(err) => log::info!("portal: not serving: {err}"),
            }
            std::process::exit(0);
        })
        .expect("spawn the portal thread");

    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            crate::init_app(&config, cx);
            crate::theme::load_bundled(cx);
            let state = Rc::new(RefCell::new(State {
                config,
                windows: HashMap::new(),
                idle: None,
            }));
            arm_idle(&state, cx);
            let loop_state = state.clone();
            cx.spawn(async move |cx| {
                while let Some(msg) = rx.next().await {
                    cx.update(|cx| handle(msg, &loop_state, cx));
                }
            })
            .detach();
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() {
                    arm_idle(&state, cx);
                }
            })
            .detach();
        });
    0
}

/// Serve the portal interface until the bus connection closes.
async fn serve(tx: mpsc::UnboundedSender<PortalMsg>) -> ashpd::backend::Result<()> {
    Builder::new(BUS_NAME)?
        // A second instance must fail instead of taking the name over.
        .with_flags(RequestNameFlags::DoNotQueue.into())
        .with_spawn(ThreadSpawner)
        .with_name_lost(|| {
            log::info!("portal: bus name lost, exiting");
            std::process::exit(0);
        })
        .file_chooser(service::VaisChooser::new(tx))
        .build()
        .await
}

/// ashpd needs an executor for two small tasks (the name-lost watcher and
/// the close callback); a thread each is plenty.
struct ThreadSpawner;

impl Spawn for ThreadSpawner {
    fn spawn_obj(&self, future: FutureObj<'static, ()>) -> Result<(), SpawnError> {
        std::thread::Builder::new()
            .name("portal-task".into())
            .spawn(move || futures::executor::block_on(future))
            .map(|_| ())
            .map_err(|_| SpawnError::shutdown())
    }
}

fn handle(msg: PortalMsg, state: &Rc<RefCell<State>>, cx: &mut App) {
    match msg {
        PortalMsg::Pick { token, spec, reply } => {
            // The picker window comes in the next step.
            log::info!("portal: no picker yet for {token}: {}", spec.method());
            let _ = &state;
            reply.send(PickResult::Cancelled);
        }
        PortalMsg::Close(token) => {
            let window = state.borrow_mut().windows.remove(&token);
            if let Some(window) = window {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

fn idle_timeout() -> Duration {
    std::env::var("VAIS_PORTAL_IDLE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_IDLE)
}

/// Exit once no window has been open for the idle time.
fn arm_idle(state: &Rc<RefCell<State>>, cx: &mut App) {
    let idle = idle_timeout();
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(idle).await;
        cx.update(|cx| {
            if cx.windows().is_empty() {
                log::info!("portal: idle for {idle:?}, exiting");
                cx.quit();
            }
        });
    });
    state.borrow_mut().idle = Some(task);
}
