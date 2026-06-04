//! fio 5 — fire-and-forget desktop notifications.
//!
//! Shells out to the platform notifier (no extra crates): `osascript` on macOS,
//! `notify-send` on Linux. A reaper thread waits on the child so we don't leak
//! zombies over a long session. Unknown platforms are a no-op.

use std::process::Command;

pub fn send(title: &str, body: &str) {
    let Some(mut cmd) = build(title, body) else { return };
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(target_os = "macos")]
fn build(title: &str, body: &str) -> Option<Command> {
    // AppleScript string literals are double-quoted; escape quotes/backslashes.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    let mut cmd = Command::new("osascript");
    cmd.arg("-e").arg(script);
    Some(cmd)
}

#[cfg(target_os = "linux")]
fn build(title: &str, body: &str) -> Option<Command> {
    let mut cmd = Command::new("notify-send");
    cmd.arg(title).arg(body);
    Some(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn build(_title: &str, _body: &str) -> Option<Command> {
    None
}
