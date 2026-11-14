//! Console input for the interactive mode: line editing and a history (kept in
//! `~/.ferrooxide_history`) when stdin is a terminal, plain lines otherwise.

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};

const PROMPT: &str = "fx> ";

pub struct Console {
    ed: Option<rustyline::DefaultEditor>,
    history: Option<PathBuf>,
}

impl Console {
    pub fn new() -> Console {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Console { ed: None, history: None };
        }
        save_terminal();
        let Ok(mut ed) = rustyline::DefaultEditor::new() else { return Console { ed: None, history: None } };
        let history = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ferrooxide_history"));
        if let Some(h) = &history {
            let _ = ed.load_history(h);
        }
        Console { ed: Some(ed), history }
    }

    /// The next line; None at the end of the input (Ctrl-D).
    pub fn read(&mut self) -> Option<String> {
        match &mut self.ed {
            Some(ed) => match ed.readline(PROMPT) {
                Ok(line) => {
                    if !line.trim().is_empty() {
                        let _ = ed.add_history_entry(line.as_str());
                        if let Some(h) = &self.history {
                            let _ = ed.save_history(h);
                        }
                    }
                    Some(line)
                }
                // Ctrl-C: an empty line
                Err(rustyline::error::ReadlineError::Interrupted) => Some(String::new()),
                Err(_) => None,
            },
            None => {
                let mut line = String::new();
                match std::io::stdin().lock().read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
                }
            }
        }
    }
}

/// With a server (`--cdsp`/`--surfer`) the console is read on its own thread: the next prompt
/// waits until the previous command's report is printed (`ready`).
static READY: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// The previous command is done: the console may prompt for the next one.
pub fn ready() {
    let (m, c) = &READY;
    *m.lock().unwrap() += 1;
    c.notify_one();
}

/// Wait for `ready` (on the console thread).
pub fn wait_ready() {
    let (m, c) = &READY;
    let mut n = m.lock().unwrap();
    while *n == 0 {
        n = c.wait(n).unwrap();
    }
    *n -= 1;
}

/// The terminal settings before the line editor changed them (it switches to raw mode while it
/// reads a line, on the console thread; an exit meanwhile would leave the terminal raw).
static TERMINAL: Mutex<Option<libc::termios>> = Mutex::new(None);

fn save_terminal() {
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(0, &mut t) } == 0 {
        *TERMINAL.lock().unwrap() = Some(t);
    }
}

/// Put the terminal back as it was (call before exiting the interactive mode).
pub fn restore_terminal() {
    if let Some(t) = TERMINAL.lock().unwrap().as_ref() {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, t);
        }
    }
}
