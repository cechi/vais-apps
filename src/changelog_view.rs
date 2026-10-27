//! The changelog in a tab of its own: opened on the first start of a new
//! version, and from the ☰ menu or Settings → About.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{IconName, Sizable as _, StyledExt as _};
use gpui_kit::*;

use crate::keymap::*;
use crate::ui::{h_flex, v_flex};

/// CHANGELOG.md from the repository root, built into the binary.
const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// The releases, newest first: the file without its title and introduction,
/// which the tab's own header replaces.
fn releases() -> &'static str {
    CHANGELOG
        .find("\n## ")
        .map_or(CHANGELOG, |start| &CHANGELOG[start + 1..])
}

pub enum ChangelogEvent {
    Close,
}

pub struct ChangelogView {
    /// The parsed Markdown. Parsing runs in the background; the view
    /// observes the state so the text shows as soon as it is ready.
    text: Entity<TextViewState>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _observe_text: Subscription,
}

impl EventEmitter<ChangelogEvent> for ChangelogView {}

impl ChangelogView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextViewState::markdown(releases(), cx));
        let observe_text = cx.observe(&text, |_, _, cx| cx.notify());
        Self {
            text,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _observe_text: observe_text,
        }
    }

    pub fn title(&self) -> String {
        t!("What's new").into()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn on_close(&mut self, _: &CloseChangelog, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ChangelogEvent::Close);
    }
}

impl Render for ChangelogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted) = (theme.background, theme.foreground, theme.muted_foreground);

        // Laid out like the settings form: a readable width in the middle.
        let page = v_flex()
            .w_full()
            .max_w(rems(48.))
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().font_semibold().text_lg().child(t!("What's new")))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("VAIS Commander {}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("changelog-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ChangelogEvent::Close))),
                    ),
            )
            .child(TextView::new(&self.text).selectable(true));

        v_flex()
            .id("changelog")
            .key_context(CHANGELOG_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close))
            .relative()
            .size_full()
            .min_h_0()
            .bg(bg)
            .text_color(fg)
            .child(
                v_flex()
                    .id("changelog-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .items_center()
                    .child(page),
            )
            .child(Scrollbar::vertical(&self.scroll).viewport_from_layout())
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` attribute would shadow the standard one.
    use super::{CHANGELOG, releases};

    /// A release is tagged from the version in Cargo.toml, so the changelog
    /// must have its section by then (an `## Unreleased` one is renamed).
    #[test]
    fn changelog_has_the_current_version() {
        let heading = format!("## {} - ", env!("CARGO_PKG_VERSION"));
        assert!(
            CHANGELOG.lines().any(|line| line.starts_with(&heading)),
            "CHANGELOG.md has no `{heading}YYYY-MM-DD` section"
        );
        assert!(releases().starts_with("## "));
    }
}
