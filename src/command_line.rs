//! The command line under the panels, as in a classic commander: a prompt
//! with the active panel's directory and an input. Enter hands the text to
//! the workspace (which runs it there), Up / Down walk the history, Escape
//! gives the focus back to the panel.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;

use crate::keymap::*;
use crate::ui::h_flex;

const HISTORY_LIMIT: usize = 100;

pub enum CommandLineEvent {
    /// Enter with a non-empty command.
    Run(String),
    /// Escape: the panel should take the focus back.
    Cancelled,
}

pub struct CommandLine {
    input: Entity<InputState>,
    /// Oldest first.
    history: Vec<String>,
    /// Position while walking the history with Up / Down; the text typed
    /// before walking is kept in `draft`.
    walking: Option<usize>,
    draft: String,
    prompt: String,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CommandLineEvent> for CommandLine {}

impl CommandLine {
    pub fn new(history: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Command…"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.run(window, cx);
                }
            },
        );
        Self {
            input,
            history,
            walking: None,
            draft: String::new(),
            prompt: String::new(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The directory shown before the input.
    pub fn set_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.prompt != prompt {
            self.prompt = prompt;
            cx.notify();
        }
    }

    /// Append `text` (quoted when it has spaces) after a separating space.
    pub fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let quoted = if text.contains(' ') || text.contains('"') {
            format!("\"{}\"", text.replace('"', "\\\""))
        } else {
            text.to_string()
        };
        let mut value = self.input.read(cx).value().to_string();
        if !value.is_empty() && !value.ends_with(' ') {
            value.push(' ');
        }
        value.push_str(&quoted);
        value.push(' ');
        self.set_text(value, window, cx);
    }

    fn set_text(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let len = value.len();
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.set_selected_range(len..len, cx);
        });
        cx.notify();
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.input.read(cx).value().trim().to_string();
        if command.is_empty() {
            return;
        }
        self.history.retain(|h| *h != command);
        self.history.push(command.clone());
        if self.history.len() > HISTORY_LIMIT {
            let excess = self.history.len() - HISTORY_LIMIT;
            self.history.drain(..excess);
        }
        self.walking = None;
        self.set_text(String::new(), window, cx);
        cx.emit(CommandLineEvent::Run(command));
    }

    fn on_prev(&mut self, _: &CommandHistoryPrev, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.walking {
            None if self.history.is_empty() => return,
            None => {
                self.draft = self.input.read(cx).value().to_string();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.walking = Some(next);
        self.set_text(self.history[next].clone(), window, cx);
    }

    fn on_next(&mut self, _: &CommandHistoryNext, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.walking else {
            return;
        };
        if current + 1 < self.history.len() {
            self.walking = Some(current + 1);
            self.set_text(self.history[current + 1].clone(), window, cx);
        } else {
            self.walking = None;
            let draft = std::mem::take(&mut self.draft);
            self.set_text(draft, window, cx);
        }
    }

    fn on_cancel(&mut self, _: &CommandLineCancel, _: &mut Window, cx: &mut Context<Self>) {
        self.walking = None;
        cx.emit(CommandLineEvent::Cancelled);
    }
}

impl Render for CommandLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, border, muted) = (
            theme.status_bar,
            theme.status_bar_border,
            theme.muted_foreground,
        );
        h_flex()
            .key_context(COMMAND_LINE_CONTEXT)
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_cancel))
            .flex_none()
            .w_full()
            .h(rems(1.9))
            .gap_2()
            .px_2()
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .text_sm()
            .child(
                div()
                    .flex_none()
                    .max_w(rems(24.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(muted)
                    .child(format!("{} ›", self.prompt)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.input).appearance(false).xsmall()),
            )
    }
}
