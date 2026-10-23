mod agents;
mod app;
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
mod ops;
mod ops_panel;
mod palette;
mod panel;
mod platform;
mod rename;
mod settings;
mod settings_view;
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
    env_logger::init();
    let config = Config::load();

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        settings::init(&config, cx);
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
