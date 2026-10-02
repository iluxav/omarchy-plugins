//! Video thumbnails. The same approach as omanote's inline images: with the
//! Kitty graphics protocol (Ghostty, Kitty) an image is sent once and placed
//! with Unicode placeholder cells, which are ordinary text to the rest of the
//! app, so scrolling and clipping just work. Elsewhere, half blocks.
//!
//! Downloading, decoding and scaling happen on worker threads; the UI asks for
//! a picture at a size and draws a placeholder until it is ready.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};
use ratatui::style::{Color, Style};

use crate::diacritics::DIACRITICS;

const PLACEHOLDER: char = '\u{10EEEE}';
const CHUNK: usize = 4096;
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const WORKERS: usize = 4;
/// Pictures kept in the terminal at once; the least recently drawn go first.
const KEEP: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Kitty,
    Blocks,
    Off,
}

/// Guess from the environment. `OMAFEED_IMAGES=kitty|blocks|off` overrides.
pub fn detect() -> Mode {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    match var("OMAFEED_IMAGES").as_str() {
        "kitty" => return Mode::Kitty,
        "blocks" => return Mode::Blocks,
        "off" => return Mode::Off,
        _ => {}
    }
    let term = var("TERM");
    let native = var("TERM_PROGRAM") == "ghostty" || term.contains("ghostty") || term.contains("kitty") || !var("KITTY_WINDOW_ID").is_empty();
    if native && var("TMUX").is_empty() { Mode::Kitty } else { Mode::Blocks }
}

/// One row of a picture: styled text runs.
pub type Row = Vec<(String, Style)>;

enum State {
    Loading,
    Ready(Vec<Row>),
    Failed,
}

struct Entry {
    id: u32,
    size: (u16, u16),
    state: State,
    used: u64,
}

struct Job {
    url: String,
    size: (u16, u16),
    cell: (u32, u32),
    mode: Mode,
}

enum Payload {
    Png(Vec<u8>),
    Rows(Vec<Row>),
}

type Done = (String, (u16, u16), Result<Payload, String>);

pub struct Thumbs {
    mode: Mode,
    cell: (u32, u32),
    entries: HashMap<String, Entry>,
    next_id: u32,
    frame: u64,
    outbox: Vec<u8>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Thumbs {
    pub fn new(mode: Mode, cell: (u16, u16)) -> Thumbs {
        let (jobs, queue) = channel::<Job>();
        let (finished, done) = channel::<Done>();
        let queue = Arc::new(Mutex::new(queue));
        let cache = crate::config::cache_dir().join("thumbs");
        for _ in 0..WORKERS {
            let (queue, finished, cache) = (queue.clone(), finished.clone(), cache.clone());
            std::thread::spawn(move || {
                loop {
                    let Ok(job) = queue.lock().map_err(drop).and_then(|q| q.recv().map_err(drop)) else { return };
                    let result = render(&job, &cache);
                    if finished.send((job.url, job.size, result)).is_err() {
                        return;
                    }
                }
            });
        }
        let mut outbox = Vec::new();
        if mode == Mode::Kitty {
            outbox.extend(b"\x1b_Ga=d,d=A,q=2\x1b\\");
        }
        Thumbs { mode, cell: (cell.0.max(1) as u32, cell.1.max(1) as u32), entries: HashMap::new(), next_id: 1, frame: 0, outbox, jobs, done }
    }

    pub fn enabled(&self) -> bool {
        self.mode != Mode::Off
    }

    /// Real pictures, not half blocks: small things like site icons are only
    /// worth drawing this way.
    pub fn kitty(&self) -> bool {
        self.mode == Mode::Kitty
    }

    /// Rows a 16:9 picture takes at this width.
    pub fn rows_for(&self, cols: u16) -> u16 {
        let (cw, ch) = self.cell;
        ((cols as u32 * cw * 9) as f64 / (16 * ch) as f64).round().clamp(1.0, DIACRITICS.len() as f64) as u16
    }

    pub fn set_cell(&mut self, cell: (u16, u16)) {
        let cell = (cell.0.max(1) as u32, cell.1.max(1) as u32);
        if cell != self.cell {
            self.cell = cell;
            for entry in self.entries.values_mut() {
                entry.size = (0, 0);
            }
        }
    }

    /// Call once per frame, before asking for pictures.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// The picture's rows if it is ready at this size; otherwise it is on its way.
    pub fn get(&mut self, url: &str, cols: u16, rows: u16) -> Option<&[Row]> {
        if self.mode == Mode::Off {
            return None;
        }
        let size = (cols, rows);
        if !self.entries.contains_key(url) {
            let id = self.next_id;
            self.next_id = (self.next_id + 1) % 0xff_ffff;
            self.entries.insert(url.to_string(), Entry { id, size: (0, 0), state: State::Loading, used: 0 });
        }
        let entry = self.entries.get_mut(url)?;
        entry.used = self.frame;
        if entry.size != size && !matches!(entry.state, State::Failed) {
            entry.size = size;
            entry.state = State::Loading;
            let _ = self.jobs.send(Job { url: url.to_string(), size, cell: self.cell, mode: self.mode });
        }
        match &self.entries.get(url)?.state {
            State::Ready(rows) => Some(rows),
            _ => None,
        }
    }

    /// Take in finished pictures. True if any arrived.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((url, size, result)) = self.done.try_recv() {
            let Some(entry) = self.entries.get_mut(&url) else { continue };
            if entry.size != size {
                continue; // asked for again at another size since
            }
            any = true;
            entry.state = match result {
                Ok(Payload::Rows(rows)) => State::Ready(rows),
                Ok(Payload::Png(png)) => {
                    transmit(&mut self.outbox, entry.id, size, &png);
                    State::Ready(placeholder_rows(entry.id, size.0, size.1))
                }
                Err(_) => State::Failed,
            };
        }
        if any {
            self.evict();
        }
        any
    }

    fn evict(&mut self) {
        let mut ready: Vec<(u64, String)> = self.entries.iter().filter(|(_, e)| matches!(e.state, State::Ready(_))).map(|(u, e)| (e.used, u.clone())).collect();
        if ready.len() <= KEEP {
            return;
        }
        ready.sort();
        let excess = ready.len() - KEEP;
        for (used, url) in ready.into_iter().take(excess) {
            if used == self.frame {
                break;
            }
            if let Some(entry) = self.entries.remove(&url) {
                if self.mode == Mode::Kitty {
                    self.outbox.extend(format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", entry.id).bytes());
                }
            }
        }
    }

    /// Escape sequences that must reach the terminal before the next frame.
    pub fn take_outbox(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.outbox)
    }
}

/// Download (or read from the cache) and fit into the box.
fn render(job: &Job, cache: &std::path::Path) -> Result<Payload, String> {
    let img = load(&job.url, cache)?;
    let (cols, rows) = (job.size.0 as u32, job.size.1 as u32);
    match job.mode {
        Mode::Kitty => {
            let (pw, ph) = (cols * job.cell.0, rows * job.cell.1);
            let mut fitted = img.resize_to_fill(pw, ph, FilterType::Triangle).to_rgba8();
            // A thumbnail gets soft corners; an icon one cell high just a hint of them.
            let radius = if job.size.1 <= 1 { 3.0 } else { (job.cell.0.min(job.cell.1) as f32 * 0.9).max(4.0) };
            round_corners(&mut fitted, radius);
            let mut png = Vec::new();
            DynamicImage::ImageRgba8(fitted).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).map_err(|e| e.to_string())?;
            Ok(Payload::Png(png))
        }
        _ => Ok(Payload::Rows(block_rows(&img, cols, rows))),
    }
}

fn load(url: &str, cache: &std::path::Path) -> Result<DynamicImage, String> {
    let file: PathBuf = cache.join(format!("{:016x}", fnv1a(url)));
    if let Ok(bytes) = std::fs::read(&file) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            return Ok(img);
        }
    }
    let fetched = fetch(url).and_then(|bytes| image::load_from_memory(&bytes).map(|img| (bytes, img)).map_err(|e| e.to_string()));
    // No `/favicon.ico`: the site's home page may name its icon instead.
    let (bytes, img) = match (fetched, url.strip_suffix("/favicon.ico")) {
        (Ok(found), _) => found,
        (Err(_), Some(site)) => {
            let page = String::from_utf8_lossy(&fetch(&format!("{site}/"))?).into_owned();
            let icon = declared_icon(&page, site).ok_or("no icon")?;
            let bytes = fetch(&icon)?;
            let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
            (bytes, img)
        }
        (Err(e), None) => return Err(e),
    };
    // Kept under the address asked for, so next time it is found at once.
    let _ = std::fs::create_dir_all(cache).and_then(|_| std::fs::write(&file, &bytes));
    Ok(img)
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let response = crate::sources::agent().get(url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The best icon a page names: the biggest one we can draw (a large icon
/// scales down crisply), skipping SVG.
fn declared_icon(html: &str, site: &str) -> Option<String> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let at = tag.find(&format!("{name}="))? + name.len() + 1;
        let quote = tag[at..].chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let rest = &tag[at + 1..];
        Some(rest[..rest.find(quote)?].replace("&amp;", "&"))
    };
    let mut best: Option<(u32, String)> = None;
    for tag in html.split("<link").skip(1).map(|t| t.split('>').next().unwrap_or("")) {
        let rel = attr(tag, "rel").unwrap_or_default().to_lowercase();
        let Some(href) = attr(tag, "href") else { continue };
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon") || href.ends_with(".svg") || tag.contains("svg+xml") {
            continue;
        }
        // "180x180" → 180; no size given: assume a small classic favicon.
        let size = attr(tag, "sizes").and_then(|s| s.split(['x', 'X']).next()?.parse().ok()).unwrap_or(if rel.contains("apple") { 180 } else { 16 });
        let url = match href {
            h if h.starts_with("http") => h,
            h if h.starts_with("//") => format!("https:{h}"),
            h if h.starts_with('/') => format!("{site}{h}"),
            h => format!("{site}/{h}"),
        };
        if best.as_ref().is_none_or(|(s, _)| size > *s) {
            best = Some((size, url));
        }
    }
    best.map(|(_, url)| url)
}

/// Where the thumbnail for `url` is cached, if it is (for notification icons).
pub fn cached(url: &str) -> Option<PathBuf> {
    let file = crate::config::cache_dir().join("thumbs").join(format!("{:016x}", fnv1a(url)));
    file.exists().then_some(file)
}

/// Fetch into the cache without drawing it.
pub fn prefetch(url: &str) -> Option<PathBuf> {
    let cache = crate::config::cache_dir().join("thumbs");
    load(url, &cache).ok()?;
    cached(url)
}

/// Transparent, antialiased corners: the terminal's background shows through.
fn round_corners(img: &mut RgbaImage, radius: f32) {
    let (w, h) = img.dimensions();
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    let span = r.ceil() as u32;
    for y in 0..span.min(h) {
        for x in 0..span.min(w) {
            // Distance from the corner circle's centre, for the top-left pixel; mirrored for the rest.
            let (dx, dy) = (r - x as f32 - 0.5, r - y as f32 - 0.5);
            let d = (dx * dx + dy * dy).sqrt();
            let coverage = (r - d + 0.5).clamp(0.0, 1.0);
            if coverage >= 1.0 {
                continue;
            }
            for (px, py) in [(x, y), (w - 1 - x, y), (x, h - 1 - y), (w - 1 - x, h - 1 - y)] {
                let p = img.get_pixel_mut(px, py);
                p.0[3] = (p.0[3] as f32 * coverage) as u8;
            }
        }
    }
}

fn transmit(out: &mut Vec<u8>, id: u32, (cols, rows): (u16, u16), png: &[u8]) {
    let payload = base64(png);
    let mut chunks = payload.as_bytes().chunks(CHUNK).peekable();
    let mut first = true;
    out.extend(format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\").bytes());
    while let Some(chunk) = chunks.next() {
        let more = chunks.peek().is_some() as u8;
        let head = if first { format!("a=T,U=1,f=100,i={id},c={cols},r={rows},q=2,m={more}") } else { format!("m={more}") };
        first = false;
        out.extend(b"\x1b_G");
        out.extend(head.bytes());
        out.push(b';');
        out.extend(chunk);
        out.extend(b"\x1b\\");
    }
}

/// One string of placeholder cells per row. The image id rides in the
/// foreground colour; two combining marks per cell carry its row and column.
fn placeholder_rows(id: u32, cols: u16, rows: u16) -> Vec<Row> {
    let style = Style::new().fg(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8));
    (0..rows as usize)
        .map(|r| {
            let mut text = String::with_capacity(cols as usize * 10);
            for c in 0..(cols as usize).min(DIACRITICS.len()) {
                text.extend([PLACEHOLDER, DIACRITICS[r], DIACRITICS[c]]);
            }
            vec![(text, style)]
        })
        .collect()
}

/// `▀` per cell: the top pixel as foreground, the bottom one as background.
fn block_rows(img: &DynamicImage, cols: u32, rows: u32) -> Vec<Row> {
    let small = img.resize_to_fill(cols, rows * 2, FilterType::Triangle).to_rgba8();
    let colour = |x: u32, y: u32| {
        let p = small.get_pixel(x, y).0;
        Color::Rgb(p[0], p[1], p[2])
    };
    (0..rows).map(|r| (0..cols).map(|x| ("▀".to_string(), Style::new().fg(colour(x, r * 2)).bg(colour(x, r * 2 + 1)))).collect()).collect()
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn corners_turn_transparent_and_the_middle_does_not() {
        let mut img = RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255]));
        round_corners(&mut img, 6.0);
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
        assert_eq!(img.get_pixel(39, 19).0[3], 0);
        assert_eq!(img.get_pixel(20, 10).0[3], 255);
        assert_eq!(img.get_pixel(20, 0).0[3], 255, "the straight edge stays");
    }

    #[test]
    fn finds_the_biggest_drawable_icon_a_page_names() {
        let page = r#"<link rel="icon" type="image/png" sizes="16x16" href="/images/favicon-16x16.png">
            <link rel="icon" type="image/png" sizes="32x32" href="https://b.org/images/favicon-32x32.png">
            <link rel="icon" type="image/svg+xml" href="/images/favicon.svg">
            <link rel="stylesheet" href="/big.css">"#;
        assert_eq!(declared_icon(page, "https://b.org").as_deref(), Some("https://b.org/images/favicon-32x32.png"));
        let apple = r#"<link rel='shortcut icon' href='f.ico'><link rel="apple-touch-icon" href="//cdn.b.org/t.png">"#;
        assert_eq!(declared_icon(apple, "https://b.org").as_deref(), Some("https://cdn.b.org/t.png"));
        assert_eq!(declared_icon("<link rel=\"icon\" href=\"/i.svg\">", "https://b.org"), None);
    }

    /// `cargo test -- --ignored live_icons`: real sites, over the network.
    #[test]
    #[ignore]
    fn live_icons() {
        let cache = std::env::temp_dir().join(format!("omafeed-icons-{}", std::process::id()));
        for url in ["https://blog.rust-lang.org/favicon.ico", "https://news.ycombinator.com/favicon.ico", "https://www.ynetnews.com/favicon.ico"] {
            let img = load(url, &cache).unwrap_or_else(|e| panic!("{url}: {e}"));
            println!("{url}: {}x{}", img.width(), img.height());
        }
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn placeholders_carry_the_id_and_position() {
        let rows = placeholder_rows(0x010203, 3, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].1.fg, Some(Color::Rgb(1, 2, 3)));
        let cells: Vec<char> = rows[1][0].0.chars().collect();
        assert_eq!(&cells[3..6], [PLACEHOLDER, DIACRITICS[1], DIACRITICS[1]]);
    }
}
