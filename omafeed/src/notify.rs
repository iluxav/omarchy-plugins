//! Desktop notifications. On Omarchy, `omarchy-notification-send` gives them
//! the item's glyph, the video's thumbnail, and a click that opens the item;
//! anywhere else, plain `notify-send`. A few new things get one each, a burst
//! gets a summary.

use std::process::{Command, Stdio};

use crate::model::{Item, Kind};

const ONE_BY_ONE: usize = 3;

pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

struct Note<'a> {
    headline: String,
    body: String,
    glyph: &'a str,
    image: Option<String>,
    urgent: bool,
    open: Option<String>,
}

pub fn new_items(items: Vec<Item>) {
    if items.is_empty() {
        return;
    }
    // Off the UI thread: a video's notification waits for its thumbnail.
    std::thread::spawn(move || {
        if items.len() > ONE_BY_ONE {
            let mails = items.iter().filter(|i| i.kind == Kind::Mail).count();
            let headline = match mails {
                0 => format!("{} new in your feed", items.len()),
                m if m == items.len() => format!("{m} new emails"),
                m => format!("{} new · {m} email{}", items.len(), if m == 1 { "" } else { "s" }),
            };
            let body: Vec<String> = items.iter().take(4).map(|i| format!("{}  {}", i.kind.icon(), i.source)).collect();
            send(Note { headline, body: body.join("\n"), glyph: "󰎟", image: None, urgent: mails > 0, open: None });
            return;
        }
        for item in items {
            let image = item.thumb.as_deref().and_then(crate::thumbs::prefetch).map(|p| p.display().to_string());
            send(Note {
                headline: item.source.clone(),
                body: item.title.clone(),
                glyph: item.kind.icon(),
                image,
                urgent: item.kind == Kind::Mail,
                open: Some(item.url.clone()),
            });
        }
    });
}

/// A new item that a prism caught: its own notification, since this is what
/// the prisms are there to find. Clicking it opens the item.
pub fn candidate(item: Item, prism: String, score: f32) {
    std::thread::spawn(move || {
        let image = item.thumb.as_deref().and_then(crate::thumbs::prefetch).map(|p| p.display().to_string());
        send(Note {
            headline: format!("{prism} · {:.0}%", score * 100.0),
            body: format!("{}\n{}", item.title, item.source),
            glyph: "󰌵",
            image,
            urgent: true,
            open: Some(item.url),
        });
    });
}

fn send(note: Note) {
    let urgency = if note.urgent { "normal" } else { "low" };
    let mut cmd;
    if on_path("omarchy-notification-send") {
        cmd = Command::new("omarchy-notification-send");
        cmd.args(["--app-name", "omafeed", "-g", note.glyph, "-u", urgency]);
        if let Some(image) = &note.image {
            cmd.args(["--image", image]);
        }
        cmd.args([&note.headline, &note.body]);
        if let Some(url) = &note.open {
            cmd.args(["--exec", browser(), url]);
        }
    } else {
        cmd = Command::new("notify-send");
        cmd.args(["-a", "omafeed", "-u", urgency]);
        if let Some(image) = &note.image {
            cmd.args(["-i", image]);
        }
        cmd.args([format!("{}  {}", note.glyph, note.headline), note.body]);
    }
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Omarchy's browser launcher also brings the browser forward in Hyprland.
pub fn browser() -> &'static str {
    if on_path("omarchy-launch-browser") { "omarchy-launch-browser" } else { "xdg-open" }
}

/// Run something detached from the terminal, and reap it when it exits.
pub fn spawn(program: &str, args: &[&str]) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
