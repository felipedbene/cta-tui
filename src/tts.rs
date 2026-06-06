//! Optional voice narration — speaks AI text via the OS text-to-speech command,
//! mirroring `notify.rs` (shell out, no crate). macOS `say`, Linux `spd-say`
//! (speech-dispatcher) falling back to `espeak`; Windows is a no-op.
//!
//! Each new utterance interrupts the previous one (kept in `CURRENT`) so the
//! dispatcher reads "live" rather than queueing a backlog.

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static CURRENT: Mutex<Option<Child>> = Mutex::new(None);

/// Speak `text`, interrupting (and reaping) any in-progress utterance.
pub fn speak(text: &str) {
    let next = spawn_tts(text);
    if let Ok(mut guard) = CURRENT.lock() {
        if let Some(mut old) = guard.take() {
            let _ = old.kill();
            let _ = old.wait(); // reap so we don't leave a zombie
        }
        *guard = next;
    }
}

fn null() -> Stdio {
    Stdio::null()
}

#[cfg(target_os = "macos")]
fn spawn_tts(text: &str) -> Option<Child> {
    Command::new("say")
        .arg(text)
        .stdin(null())
        .stdout(null())
        .stderr(null())
        .spawn()
        .ok()
}

#[cfg(target_os = "linux")]
fn spawn_tts(text: &str) -> Option<Child> {
    // `--` guards text that begins with a dash.
    Command::new("spd-say")
        .args(["-w", "--"]) // -w: wait flag so kill() actually stops it
        .arg(text)
        .stdin(null())
        .stdout(null())
        .stderr(null())
        .spawn()
        .or_else(|_| {
            Command::new("espeak")
                .arg(text)
                .stdin(null())
                .stdout(null())
                .stderr(null())
                .spawn()
        })
        .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn spawn_tts(_text: &str) -> Option<Child> {
    None
}
