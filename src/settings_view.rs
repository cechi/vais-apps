//! The application settings, shown in a tab of its own. The view keeps a
//! copy of the values and tells the workspace about changes through
//! [`SettingsEvent`]; the workspace pushes the current values back with
//! [`SettingsView::sync`] when they change elsewhere (Ctrl+= etc.).

use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{IconName, Sizable as _, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::{Config, ThemeChoice};
use crate::keymap::*;
use crate::settings::DropAction;
use crate::theme;
use crate::ui::{h_flex, v_flex};

pub enum SettingsEvent {
    /// Open the theme chooser.
    ChooseTheme,
    Scale(f32),
    Sidebar(bool),
    DropAction(DropAction),
    Close,
}

/// What the view shows; the workspace owns the real values.
#[derive(Clone, PartialEq)]
pub struct SettingsValues {
    pub theme: ThemeChoice,
    pub scale: f32,
    pub sidebar_open: bool,
    pub drop_action: DropAction,
}

pub struct SettingsView {
    values: SettingsValues,
    slider: Entity<SliderState>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(
        values: SettingsValues,
        (min, max, step): (f32, f32, f32),
        cx: &mut Context<Self>,
    ) -> Self {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(step)
                .default_value(values.scale)
        });
        let subscription = cx.subscribe(&slider, |this, _, event, cx| {
            if let SliderEvent::Change(value) = event {
                this.values.scale = value.start();
                cx.emit(SettingsEvent::Scale(value.start()));
            }
        });
        Self {
            values,
            slider,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn title(&self) -> String {
        "Settings".into()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    /// The workspace changed something (keyboard shortcut, status bar, theme
    /// dialog); show the new values.
    pub fn sync(&mut self, values: SettingsValues, cx: &mut Context<Self>) {
        if self.values != values {
            self.values = values;
            cx.notify();
        }
    }

    fn on_close(&mut self, _: &CloseSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Close);
    }

    fn set_drop_action(&mut self, action: DropAction, cx: &mut Context<Self>) {
        if self.values.drop_action != action {
            self.values.drop_action = action;
            cx.emit(SettingsEvent::DropAction(action));
            cx.notify();
        }
    }

    // ----- rendering -------------------------------------------------------

    fn section(title: &'static str, muted: Hsla, border: Hsla) -> Div {
        v_flex()
            .gap_3()
            .pb_4()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child(title.to_uppercase()),
            )
    }

    /// A row: label and description on the left, the control on the right.
    fn row(label: &'static str, help: &'static str, muted: Hsla, control: AnyElement) -> Div {
        Self::row_with_path(label, help, None, muted, control)
    }

    /// Like [`Self::row`] with a (truncated) path under the description.
    fn row_with_path(
        label: &'static str,
        help: &'static str,
        path: Option<PathBuf>,
        muted: Hsla,
        control: AnyElement,
    ) -> Div {
        h_flex()
            .gap_4()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().child(label))
                    .child(div().text_xs().text_color(muted).child(help))
                    .when_some(path, |col, path| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .truncate()
                                .child(path.display().to_string()),
                        )
                    }),
            )
            .child(div().flex_none().child(control))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted, border) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
        );
        // Keep the slider at the workspace's scale (set_value emits nothing).
        let shown = self.slider.read(cx).value().start();
        if (shown - self.values.scale).abs() > 0.001 {
            let scale = self.values.scale;
            self.slider
                .update(cx, |s, cx| s.set_value(scale, window, cx));
        }

        let theme_name = match &self.values.theme {
            ThemeChoice::System => "System".to_string(),
            ThemeChoice::Light => "Default Light".to_string(),
            ThemeChoice::Dark => "Default Dark".to_string(),
            ThemeChoice::Named(name) => name.clone(),
        };
        let config_path = Config::path().unwrap_or_default();
        let themes_dir = theme::themes_dir().unwrap_or_default();
        let drop_buttons = [
            (DropAction::Ask, "Ask"),
            (DropAction::Move, "Move"),
            (DropAction::Copy, "Copy"),
        ]
        .into_iter()
        .map(|(action, label)| {
            let button = Button::new(("drop-action", action as usize))
                .small()
                .label(label);
            let button = if action == self.values.drop_action {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, _, cx| this.set_drop_action(action, cx)))
        });

        let scale_label = format!("{:.0} %", self.values.scale * 100.0);
        let open_dir = |dir: PathBuf| {
            move |_: &ClickEvent, _: &mut Window, _: &mut App| {
                let _ = std::fs::create_dir_all(&dir);
                if let Err(err) = open::that_detached(&dir) {
                    log::warn!("open {}: {err}", dir.display());
                }
            }
        };

        v_flex()
            .id("settings")
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .bg(bg)
            .text_color(fg)
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().font_semibold().text_lg().child("Settings"))
                    .child(div().flex_1())
                    .child(
                        Button::new("settings-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    ),
            )
            .child(
                Self::section("Appearance", muted, border)
                    .child(Self::row(
                        "Theme",
                        "Built-in light / dark, System, or a theme from the themes directory",
                        muted,
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().child(theme_name))
                            .child(
                                Button::new("settings-theme")
                                    .small()
                                    .label("Choose…")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(SettingsEvent::ChooseTheme)
                                    })),
                            )
                            .into_any_element(),
                    ))
                    .child(Self::row(
                        "UI scale",
                        "Size of everything; Ctrl+= / Ctrl+- / Ctrl+0 do the same",
                        muted,
                        h_flex()
                            .gap_2()
                            .child(Slider::new(&self.slider).w(rems(12.)))
                            .child(div().w(rems(3.)).text_right().text_sm().child(scale_label))
                            .into_any_element(),
                    ))
                    .child(Self::row(
                        "Places sidebar",
                        "Show user folders and volumes at the left (Ctrl+B)",
                        muted,
                        Checkbox::new("settings-sidebar")
                            .checked(self.values.sidebar_open)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.values.sidebar_open = *checked;
                                cx.emit(SettingsEvent::Sidebar(*checked));
                                cx.notify();
                            }))
                            .into_any_element(),
                    )),
            )
            .child(Self::section("Files", muted, border).child(Self::row(
                "Drag and drop",
                "What dropping entries onto a panel does",
                muted,
                h_flex().gap_1().children(drop_buttons).into_any_element(),
            )))
            .child(
                Self::section("Locations", muted, border)
                    .child(Self::row_with_path(
                        "Config file",
                        "Saved automatically: tabs, layouts, theme, scale",
                        Some(config_path.clone()),
                        muted,
                        Button::new("settings-open-config")
                            .small()
                            .label("Open folder")
                            .on_click(open_dir(
                                config_path.parent().map(PathBuf::from).unwrap_or_default(),
                            ))
                            .into_any_element(),
                    ))
                    .child(Self::row_with_path(
                        "Themes directory",
                        "JSON theme files here are watched and offered in the theme chooser",
                        Some(themes_dir.clone()),
                        muted,
                        Button::new("settings-open-themes")
                            .small()
                            .label("Open folder")
                            .on_click(open_dir(themes_dir))
                            .into_any_element(),
                    )),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(muted)
                            .child("ABOUT"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(format!("VAIS Commander {}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("VAIS = vibecoded AI slop · MIT licence"),
                    )
                    .child(
                        Button::new("settings-github")
                            .link()
                            .small()
                            .label("github.com/cechi/vais-commander")
                            .on_click(|_, _, _| {
                                let _ =
                                    open::that_detached("https://github.com/cechi/vais-commander");
                            }),
                    ),
            )
    }
}
