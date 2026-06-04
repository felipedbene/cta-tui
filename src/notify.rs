//! fio 5 — fire-and-forget desktop notifications.
//!
//! Shells out to the platform notifier (no extra crates). A reaper thread waits
//! on the child so we don't leak zombies over a long session.
//!
//! Icon: macOS `osascript display notification` has no icon parameter — the icon
//! is the app running the script. So `CTA_NOTIFY_ICON` (default "🚇") is used two
//! ways: a short emoji/text is prefixed to the title (reads as an icon), while a
//! filesystem path is passed to `terminal-notifier -appIcon` for a real custom
//! icon when that tool is installed (falling back to osascript otherwise).

use std::process::Command;

fn reap(mut cmd: Command) -> bool {
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

pub fn send(title: &str, body: &str) {
    let icon = std::env::var("CTA_NOTIFY_ICON").unwrap_or_else(|_| "🚇".into());
    dispatch(title, body, &icon);
}

#[cfg(target_os = "macos")]
fn dispatch(title: &str, body: &str, icon: &str) {
    // An icon that's a path → try terminal-notifier for a real custom icon.
    if icon.contains('/') {
        let mut tn = Command::new("terminal-notifier");
        tn.args(["-title", title, "-message", body, "-appIcon", icon]);
        if reap(tn) {
            return;
        }
        // terminal-notifier not installed → fall through to a plain osascript.
    }
    let titled = if icon.is_empty() || icon.contains('/') {
        title.to_string()
    } else {
        format!("{icon} {title}")
    };
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(&titled)
    );
    let mut cmd = Command::new("osascript");
    cmd.arg("-e").arg(script);
    reap(cmd);
}

#[cfg(target_os = "linux")]
fn dispatch(title: &str, body: &str, icon: &str) {
    let mut cmd = Command::new("notify-send");
    if icon.contains('/') {
        cmd.arg("-i").arg(icon);
        cmd.arg(title);
    } else {
        cmd.arg(format!("{icon} {title}").trim());
    }
    cmd.arg(body);
    reap(cmd);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn dispatch(_title: &str, _body: &str, _icon: &str) {}
