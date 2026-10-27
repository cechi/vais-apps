#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod agents;
mod app;
mod bridge;
mod changelog_view;
mod clipboard;
mod command_line;
mod config;
mod connections;
mod connections_view;
mod editor_view;
mod fs;
mod grid_view;
mod icons;
mod keymap;
mod layout;
mod mcp;
mod ops;
mod ops_panel;
mod palette;
mod panel;
mod platform;
#[cfg(target_os = "linux")]
mod portal;
mod rename;
mod settings;
mod settings_view;
#[cfg(windows)]
mod shell_menu;
mod sidebar;
mod ssh_config;
mod tab_color;
mod tabs;
mod terminal;
mod theme;
mod thumbs;
mod ui;
mod vfs;
mod watch;

use gpui_kit::*;

use crate::app::Workspace;
use crate::config::Config;

fn main() {
    // A GUI program on Windows: no console window of its own. One that
    // started us from a console (for the logs) is attached instead; the
    // stdio pipes of `--mcp` are inherited either way.
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    env_logger::init();
    // `vais-commander --mcp`: the MCP server a coding agent talks to; it
    // forwards every tool call to the running app.
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        std::process::exit(mcp::run());
    }
    // The agents start `--mcp` from this path; taken before the binary can
    // be replaced under the running app.
    agents::launch::remember_exe();
    let config = Config::load();
    // `vais-commander --portal`: the file chooser backend of xdg-desktop-portal
    // (a separate process without a main window).
    if std::env::args().nth(1).as_deref() == Some("--portal") {
        #[cfg(target_os = "linux")]
        std::process::exit(portal::run(config));
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("--portal is available on Linux only");
            std::process::exit(2);
        }
    }

    application().with_assets(assets::Assets).run(move |cx| {
        init_app(&config, cx);

        let window_bounds = config.window.map(|w| {
            let bounds = Bounds {
                origin: point(px(w.x), px(w.y)),
                size: size(px(w.w), px(w.h)),
            };
            if w.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            }
        });
        let options = WindowOptions {
            window_bounds,
            titlebar: Some(TitlebarOptions {
                title: Some("VAIS Commander".into()),
                ..Default::default()
            }),
            app_id: Some("vais-commander".into()),
            window_min_size: Some(size(px(640.), px(400.))),
            ..Default::default()
        };

        let theme = config.theme.clone();
        let font_family = config.font_family.clone();
        open_window(options, cx, move |window, cx| {
            theme::load_bundled(cx);
            theme::apply(&theme, Some(window), cx);
            theme::apply_font(font_family.as_deref(), cx);
            let workspace = cx.new(|cx| Workspace::new(config, window, cx));
            workspace.update(cx, |ws, cx| ws.focus_active(window, cx));
            workspace
        })
        .expect("failed to open window");

        // Closing the last window ends the process (and triggers the quit save).
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}

/// The globals every window of the app needs (the main window and the portal
/// picker alike).
pub(crate) fn init_app(config: &Config, cx: &mut App) {
    init(cx);
    i18n::set_language(config.language.as_deref());
    settings::init(config, cx);
    clipboard::init(cx);
    thumbs::init(cx);
    icons::init(cx);
    connections::init(config.connections.clone(), cx);
    watch::init(cx);
    cx.background_spawn(async { vfs::archive::prune_cache() })
        .detach();
    cx.bind_keys(keymap::bindings());
    // File lists are long; keep their scrollbars visible instead of fading out.
    gpui_kit::component::theme::Theme::set_scrollbar_mode(
        gpui_kit::component::scroll::ScrollbarMode::Always,
        cx,
    );
}
