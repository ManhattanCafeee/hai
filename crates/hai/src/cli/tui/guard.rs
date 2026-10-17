//! Terminal state and input, each bound to a scope: raw mode, the alternate screen and mouse capture
//! are restored when the guard drops (including on the early returns of `?`), and the reader's thread
//! stops with the reader. Release builds abort on panic (`[profile.release]` sets `panic = "abort"`),
//! so a panic there is the one exit that leaves the terminal as it was.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossterm::event::{Event as TermEvent, poll, read};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::error::{AppError, Result};

/// How long the reader sleeps before it looks at its stop flag again. An event wakes it at once, so
/// this bounds only how long its thread outlives the reader.
const STOP_CHECK: Duration = Duration::from_millis(100);

pub(super) struct TerminalGuard;

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        use crossterm::{
            event::EnableMouseCapture,
            terminal::{EnterAlternateScreen, disable_raw_mode, enable_raw_mode},
        };
        enable_raw_mode()?;
        let res = crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture);
        if let Err(e) = res {
            let _ = disable_raw_mode();
            return Err(e.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        use std::io::Write;

        use crossterm::{
            event::DisableMouseCapture,
            terminal::{LeaveAlternateScreen, disable_raw_mode},
        };
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(out, DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let _ = out.flush();
    }
}

/// Terminal events, read on a thread of their own and handed over as they land. crossterm's own
/// `EventStream` only produces an event when the loop happens to poll it, which ties input to the
/// tick: one key per tick, so holding a key or pasting a line crawls while the process sits idle.
/// Reading in a loop instead lets a keypress wake the loop the moment it arrives.
pub(super) struct InputReader {
    events: UnboundedReceiver<TermEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InputReader {
    pub fn spawn() -> Result<Self> {
        let (sender, events) = unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name(String::from("hai-tui-input"))
            .spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match poll(STOP_CHECK) {
                        // The timeout is only the stop check; readiness returns at once.
                        Ok(true) => match read() {
                            Ok(event) => {
                                if sender.send(event).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        },
                        Ok(false) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| AppError::message(format!("spawning the TUI input thread: {e}")))?;
        Ok(Self {
            events,
            stop,
            thread: Some(thread),
        })
    }

    /// The next terminal event, or `None` once the reading has stopped.
    pub async fn next(&mut self) -> Option<TermEvent> {
        self.events.recv().await
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
