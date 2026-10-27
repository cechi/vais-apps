//! Spawning a program on a pseudo-terminal: the alacritty_terminal `Term`
//! behind a fair mutex, its PTY reader thread, and the channel its events
//! reach the UI through.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Notifier};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Options, Shell};
use futures::channel::mpsc::UnboundedSender;

/// What to run: a program with arguments, where and with which extra
/// environment.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

/// Forwards terminal events from the PTY thread to the UI task.
#[derive(Clone)]
pub struct EventProxy(pub UnboundedSender<Event>);

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        self.0.unbounded_send(event).ok();
    }
}

pub struct Session {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    pub notifier: Notifier,
}

/// Start `launch` on a PTY of `cols` x `rows` cells.
pub fn spawn(launch: &Launch, cols: usize, rows: usize, proxy: EventProxy) -> io::Result<Session> {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("TERM_PROGRAM".into(), "vais-commander".into());
    for (key, value) in &launch.env {
        env.insert(key.clone(), value.clone());
    }
    let options = Options {
        shell: Some(Shell::new(launch.program.clone(), launch.args.clone())),
        working_directory: launch.cwd.clone(),
        drain_on_exit: false,
        env,
        #[cfg(windows)]
        escape_args: true,
    };
    let size = WindowSize {
        num_lines: rows.max(1) as u16,
        num_cols: cols.max(1) as u16,
        cell_width: 8,
        cell_height: 16,
    };
    let pty = tty::new(&options, size, 0)?;
    let config = Config {
        scrolling_history: 10_000,
        ..Config::default()
    };
    let term = Term::new(
        config,
        &TermSize::new(cols.max(1), rows.max(1)),
        proxy.clone(),
    );
    let term = Arc::new(FairMutex::new(term));
    let event_loop = EventLoop::new(term.clone(), proxy, pty, false, false)?;
    let notifier = Notifier(event_loop.channel());
    event_loop.spawn();
    Ok(Session { term, notifier })
}
