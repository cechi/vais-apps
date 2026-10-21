mod app;
mod config;
mod fs;
mod grid_view;
mod keymap;
mod layout;
mod ops;
mod ops_panel;
mod panel;
mod platform;
mod tabs;
mod ui;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

use crate::app::Workspace;
use crate::config::{Config, ThemeChoice};

fn main() {
    env_logger::init();
    let config = Config::load();

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        cx.bind_keys(keymap::bindings());

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

        let theme = config.theme;
        open_window(options, cx, move |window, cx| {
            match theme {
                ThemeChoice::Light => Theme::change(ThemeMode::Light, Some(window), cx),
                ThemeChoice::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
                ThemeChoice::System => Theme::sync_system_appearance(Some(window), cx),
            }
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
