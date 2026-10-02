mod app;
mod body;
mod prism;
mod config;
mod diacritics;
mod imap;
#[allow(dead_code)] // copied from omanote whole
mod layout;
#[allow(dead_code)] // copied from omanote whole
mod look;
#[allow(dead_code)] // copied from omanote whole
mod markdown;
mod model;
mod notify;
mod sources;
mod store;
#[allow(dead_code)] // copied from omanote whole
mod table;
#[allow(dead_code)] // copied from omanote whole
mod theme;
mod thumbs;
mod ui;

use std::io::{Write, stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, window_size};

use app::{App, FILTERS};

const HELP: &str = "omafeed — your mail, YouTube and feeds, live in the terminal

usage:
  omafeed                          open the feed
  omafeed --add <url>              follow a feed, a blog, or a YouTube channel page
  omafeed --import-youtube <csv>   follow every channel in a Takeout subscriptions.csv

keys:
  j k ↑ ↓      move            ␣        expand / collapse
  ⏎            done (mark read / clear)
  ⇧⏎ o         open in browser p        play video in mpv
  ^Z u         undo the last ⏎ (a mail is marked unread again)
  b            pin / unpin (pins live in their own tab)
  l            links in the item (or click a link)
  s            save to omanote inbox
  ⇥ 1-5        filter          r        refresh
  ^P /         look through a prism: the best candidates first (esc clears it)
  g G          top / bottom    q        quit

settings: ~/.config/omafeed/config.toml and ~/.config/omafeed/feeds";

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("-h" | "--help") => return Ok(println!("{HELP}")),
        Some("--add") => return cli_add(args.get(1)),
        Some("--import-youtube") => return cli_import(args.get(1)),
        Some(other) => {
            eprintln!("omafeed: unknown argument {other}\n\n{HELP}");
            std::process::exit(2);
        }
    }

    let config = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("omafeed: {e}");
            std::process::exit(1);
        }
    };

    enable_raw_mode()?;
    // Ask the terminal for its colours before anything else reads the keyboard.
    theme::init(theme::detect());
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, SetTitle("omafeed"))?;
    // Shift+Enter is only told apart from Enter with the Kitty keyboard protocol (Ghostty, Kitty).
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
        ENHANCED.store(true, Ordering::Relaxed);
    }
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    stop_on_signals();

    let (tx, rx) = channel();
    let thumbs = thumbs::Thumbs::new(thumbs::detect(), cell_pixels());
    let mut app = App::new(config, thumbs, tx);
    app.select(0);
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    while !app.quit && !STOP.load(Ordering::Relaxed) {
        while let Ok(msg) = rx.try_recv() {
            app.receive(msg);
        }
        app.thumbs.poll();
        // Pictures must be in the terminal before the frame that shows them.
        let pending = app.thumbs.take_outbox();
        if !pending.is_empty() {
            let mut out = stdout();
            out.write_all(&pending)?;
            out.flush()?;
        }
        terminal.draw(|f| ui::draw(f, &mut app))?;
        app.save_if_due(false);

        // Short while something moves (the spinner), relaxed otherwise.
        let wait = if app.busy() || !app.loading.is_empty() { Duration::from_millis(80) } else { Duration::from_millis(250) };
        // The waiting is done here, not in crossterm: once the terminal has
        // hung up, crossterm's own wait spins forever and never returns.
        match wait_for_input(wait) {
            Tty::Gone => break,
            Tty::Idle => continue,
            Tty::Ready => {}
        }
        if !event::poll(Duration::ZERO)? {
            continue;
        }
        loop {
            handle(&mut app, event::read()?);
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }

    app.save_if_due(true);
    restore();
    Ok(())
}

fn handle(app: &mut App, ev: Event) {
    match ev {
        Event::Key(key) if key.kind != KeyEventKind::Release && app.palette.is_some() => app.palette_key(key),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let sup = key.modifiers.contains(KeyModifiers::SUPER);
            match key.code {
                // Super+P too, for when Hyprland lets it through (Omarchy binds it to "Pseudo window").
                KeyCode::Char('p') if ctrl || sup => app.open_palette(),
                KeyCode::Char('/') => app.open_palette(),
                KeyCode::Char('c') if ctrl => app.quit = true,
                KeyCode::Char('z') if ctrl || sup => app.undo(),
                KeyCode::Char('u') if !ctrl => app.undo(),
                KeyCode::Char('d') if ctrl => app.page(true),
                KeyCode::Char('u') if ctrl => app.page(false),
                KeyCode::Char('q') => app.quit = true,
                KeyCode::Char('j') | KeyCode::Down => app.down(),
                KeyCode::Char('k') | KeyCode::Up => app.up(),
                KeyCode::PageDown => app.page(true),
                KeyCode::PageUp => app.page(false),
                KeyCode::Char('g') | KeyCode::Home => {
                    app.scroll = 0;
                    app.select(0);
                }
                KeyCode::Char('G') | KeyCode::End => app.select(usize::MAX),
                KeyCode::Char(' ') => app.toggle(),
                KeyCode::Esc if app.prism.is_some() => app.set_prism(None),
                KeyCode::Esc => app.collapse(),
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => app.open(),
                KeyCode::Char('o') => app.open(),
                KeyCode::Enter | KeyCode::Char('m') => app.done(),
                KeyCode::Char('b') => app.pin(),
                KeyCode::Char('l') => app.open_links(),
                KeyCode::Char('p') => app.play(),
                KeyCode::Char('s') => app.save(),
                KeyCode::Char('r') => app.refresh(),
                KeyCode::Tab => app.cycle_filter(false),
                KeyCode::BackTab => app.cycle_filter(true),
                KeyCode::Char(c @ '1'..='5') => app.set_filter(FILTERS[c as usize - '1' as usize]),
                _ => {}
            }
        }
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollDown => app.wheel(true),
            MouseEventKind::ScrollUp => app.wheel(false),
            // A link opens; anywhere else on an item selects it, or opens it up.
            MouseEventKind::Down(MouseButton::Left) if app.link_at(m.column, m.row).is_some() => {
                let url = app.link_at(m.column, m.row).unwrap_or_default();
                app.open_url(&url);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(at) = app.item_at(m.row) {
                    if at == app.selected {
                        app.toggle();
                    } else {
                        app.select(at);
                    }
                }
            }
            _ => {}
        },
        Event::Resize(..) => {
            app.thumbs.set_cell(cell_pixels());
            app.bodies.clear();
            app.snap = true;
        }
        _ => {}
    }
}

fn cli_add(url: Option<&String>) -> std::io::Result<()> {
    let Some(url) = url.filter(|u| u.starts_with("http")) else {
        eprintln!("usage: omafeed --add <feed url>");
        std::process::exit(2);
    };
    eprintln!("Checking {url}…");
    let (feed, title, entries) = match sources::discover(url) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("Not added: {e}");
            std::process::exit(1);
        }
    };
    let name = if title.is_empty() { feed.clone() } else { title.clone() };
    match config::add_feeds(&[(feed.clone(), title)])? {
        0 => println!("Already following {name}"),
        _ if entries == 0 && feed.contains("youtube.com/feeds/") => {
            println!("Following {name} → {feed}\nYouTube's feed server is not answering right now; the videos show up once it is back.")
        }
        _ => println!("Following {name} ({entries} entr{}) → {feed}", if entries == 1 { "y" } else { "ies" }),
    }
    Ok(())
}

fn cli_import(file: Option<&String>) -> std::io::Result<()> {
    let Some(file) = file else {
        eprintln!("usage: omafeed --import-youtube <Takeout subscriptions.csv>");
        std::process::exit(2);
    };
    let feeds = config::youtube_takeout(&std::fs::read_to_string(file)?);
    if feeds.is_empty() {
        eprintln!("No channels found in {file}: expected Google Takeout's YouTube subscriptions.csv");
        std::process::exit(1);
    }
    let added = config::add_feeds(&feeds)?;
    println!("Following {added} new channel{} ({} in the file) → {}", if added == 1 { "" } else { "s" }, feeds.len(), config::feeds_file().display());
    Ok(())
}

/// Pixel size of one cell, for sizing pictures. A typical 1:2 cell when the terminal will not say.
fn cell_pixels() -> (u16, u16) {
    match window_size() {
        Ok(s) if s.width > 0 && s.height > 0 && s.columns > 0 && s.rows > 0 => (s.width / s.columns, s.height / s.rows),
        _ => (10, 20),
    }
}

fn restore() {
    let mut out = stdout();
    let _ = out.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\");
    if ENHANCED.load(Ordering::Relaxed) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

static STOP: AtomicBool = AtomicBool::new(false);
static ENHANCED: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// A closed window or a `kill` ends the loop cleanly, state saved.
fn stop_on_signals() {
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        unsafe { libc::signal(signal, stop as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
    // Should the loop ever be stuck when told to stop, do not linger in the
    // background: put the terminal back and go. (State is saved every few seconds.)
    std::thread::spawn(|| {
        while !STOP.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_secs(3));
        restore();
        std::process::exit(1);
    });
}

enum Tty {
    /// Input is waiting.
    Ready,
    /// Nothing came in time.
    Idle,
    /// The window was closed under us.
    Gone,
}

fn wait_for_input(wait: Duration) -> Tty {
    let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    let ready = unsafe { libc::poll(&mut fd, 1, wait.as_millis() as libc::c_int) };
    if ready < 0 {
        // A signal cut the wait short: look again next time round.
        return Tty::Idle;
    }
    match fd.revents {
        r if r & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 => Tty::Gone,
        r if r & libc::POLLIN != 0 => Tty::Ready,
        _ => Tty::Idle,
    }
}
