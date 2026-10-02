//! The feed's state and everything that changes it: arriving items, keys, clicks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use ratatui::text::Line;

use crate::prism::{self, Prisms};
use crate::config::Config;
use crate::model::{Item, Kind};
use crate::sources::{Cmd, GMAIL, Msg};
use crate::store::{self, State};
use crate::thumbs::Thumbs;
use crate::{notify, sources};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Only(Kind),
    Pinned,
}

pub const FILTERS: [Filter; 5] = [Filter::All, Filter::Only(Kind::Mail), Filter::Only(Kind::Video), Filter::Only(Kind::Post), Filter::Pinned];

/// The box that opens over the feed: what is typed, which line is picked,
/// and what it is a list of.
pub struct Palette {
    pub query: String,
    pub selected: usize,
    pub kind: PaletteKind,
}

pub enum PaletteKind {
    Prisms,
    /// The links of one item: (what it says, where it goes).
    Links(Vec<(String, String)>),
}

/// One line of the box.
pub struct Entry {
    pub label: String,
    /// Quiet, on the right: a count, a web address.
    pub detail: String,
    /// What picking it does: the prism, or the address to open.
    pub value: String,
}

/// Where an item was last drawn, in rows of the whole list: for scrolling and clicks.
#[derive(Clone, Copy)]
pub struct Span {
    pub top: u16,
    pub bottom: u16,
}

pub struct App {
    pub config: Config,
    pub state: State,
    /// On the very first run everything already there counts as seen.
    first_run: bool,
    /// Origins that have synced at least once since we started.
    synced: HashSet<String>,
    pub filter: Filter,
    pub selected: usize,
    selected_id: Option<String>,
    pub expanded: HashSet<String>,
    pub scroll: u16,
    /// Bring the selected item into view on the next frame.
    pub snap: bool,
    pub spans: Vec<Span>,
    /// Screen rows of the list, for clicks.
    pub list_area: ratatui::layout::Rect,
    pub busy_mail: bool,
    pub busy_feeds: bool,
    pub errors: HashMap<String, String>,
    toast: Option<(String, Instant)>,
    pub thumbs: Thumbs,
    pub bodies: HashMap<(String, u16), Vec<(Line<'static>, Vec<crate::ui::Hit>)>>,
    gmail: Option<Sender<Cmd>>,
    feeds: Option<Sender<Cmd>>,
    tx: Sender<Msg>,
    /// Descriptions on their way, by item id.
    pub loading: HashSet<String>,
    /// Prisms from `prisms.yaml`, when jev is set up.
    pub prisms: Option<Arc<Prisms>>,
    scorer: Option<Sender<Item>>,
    /// Sent to jev, answer not back yet (or failed: not asked again this run).
    asked: HashSet<String>,
    /// Looking through this prism: only its candidates, best first.
    pub prism: Option<String>,
    /// Arrived while we were running: a strong prism match among these is worth a notification.
    arrivals: HashSet<String>,
    pub palette: Option<Palette>,
    /// What `enter` cleared, newest last, for undo: the item as it was, and whether it was pinned.
    undo: Vec<(Item, bool)>,
    /// Links drawn this frame: (screen row, first column, end column, address).
    pub links: Vec<(u16, u16, u16, String)>,
    dirty: Option<Instant>,
    pub tick: u64,
    pub quit: bool,
}

impl App {
    pub fn new(config: Config, thumbs: Thumbs, tx: Sender<Msg>) -> App {
        let (state, first_run) = match store::load() {
            Some(state) => (state, false),
            None => (State::default(), true),
        };
        let gmail = config.gmail.clone().map(|g| sources::gmail(g, tx.clone()));
        let feeds = Some(sources::feeds(config.feed.clone(), tx.clone()));
        let mut state = state;
        let mut notice = None;
        let (prisms, scorer) = match (config.jev.key(), prism::load()) {
            (Some(key), Ok(prisms)) => {
                let prisms = Arc::new(prisms);
                // The prisms changed since these were looked at: look at everything again.
                if state.prisms != prisms.signature {
                    state.prisms = prisms.signature.clone();
                    for item in state.items.iter_mut().chain(state.pinned.iter_mut()) {
                        item.scores.clear();
                    }
                }
                let worker = prism::spawn(prisms.clone(), key, config.jev.model.clone(), tx.clone());
                (Some(prisms), Some(worker))
            }
            (Some(_), Err(e)) => {
                notice = Some(format!("Prisms are off: {e}"));
                (None, None)
            }
            (None, _) => (None, None),
        };
        let mut app = App {
            config,
            state,
            first_run,
            synced: HashSet::new(),
            filter: Filter::All,
            selected: 0,
            selected_id: None,
            expanded: HashSet::new(),
            scroll: 0,
            snap: true,
            spans: Vec::new(),
            list_area: Default::default(),
            busy_mail: false,
            busy_feeds: false,
            errors: HashMap::new(),
            toast: None,
            thumbs,
            bodies: HashMap::new(),
            gmail,
            feeds,
            tx,
            loading: HashSet::new(),
            prisms,
            scorer,
            asked: HashSet::new(),
            prism: None,
            arrivals: HashSet::new(),
            palette: None,
            undo: Vec::new(),
            links: Vec::new(),
            dirty: None,
            tick: 0,
            quit: false,
        };
        app.forget_dropped_sources();
        app.sort();
        app.score_new();
        if let Some(notice) = notice {
            app.say(notice);
        }
        app
    }

    /// Send jev whatever it has not looked at yet.
    fn score_new(&mut self) {
        let Some(scorer) = &self.scorer else { return };
        let hidden = &self.state.hidden;
        for item in self.state.items.iter().chain(&self.state.pinned) {
            if item.scores.is_empty() && !hidden.contains(&item.id) && self.asked.insert(item.id.clone()) {
                let _ = scorer.send(item.clone());
            }
        }
    }

    /// The prisms this item passes, strongest first.
    pub fn matches(&self, item: &Item) -> Vec<(String, f32)> {
        let Some(prisms) = &self.prisms else { return Vec::new() };
        let mut hits: Vec<(String, f32)> = item.scores.iter().filter(|(_, s)| **s >= prisms.threshold).map(|(n, s)| (n.clone(), *s)).collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits
    }

    pub fn has_sources(&self) -> bool {
        self.config.gmail.is_some() || !crate::config::feeds().is_empty()
    }

    /// Items of feeds no longer in the feeds file, or of Gmail no longer set up, go.
    fn forget_dropped_sources(&mut self) {
        let feeds: HashSet<String> = crate::config::feeds().into_iter().map(|(u, _)| u).collect();
        let gmail = self.config.gmail.is_some();
        self.state.items.retain(|i| if i.origin == GMAIL { gmail } else { feeds.contains(&i.origin) });
    }

    fn sort(&mut self) {
        self.state.items.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
    }

    /// What the list shows, newest first; through a prism, best candidates first.
    /// Pinned items live in their own tab and nowhere else.
    pub fn visible(&self) -> Vec<&Item> {
        let mut list: Vec<&Item> = match self.filter {
            Filter::Pinned => self.state.pinned.iter().filter(|i| self.in_prism(i)).collect(),
            Filter::All => self.state.items.iter().filter(|i| self.in_feed(i)).collect(),
            Filter::Only(kind) => self.state.items.iter().filter(|i| i.kind == kind && self.in_feed(i)).collect(),
        };
        if let Some(name) = &self.prism {
            let score = |i: &Item| i.scores.get(name).copied().unwrap_or(0.0);
            // Stable: equal scores stay newest first.
            list.sort_by(|a, b| score(b).total_cmp(&score(a)));
        }
        list
    }

    fn in_feed(&self, item: &Item) -> bool {
        !self.state.hidden.contains(&item.id) && !self.is_pinned(&item.id) && self.in_prism(item)
    }

    fn in_prism(&self, item: &Item) -> bool {
        let (Some(name), Some(prisms)) = (&self.prism, &self.prisms) else { return true };
        item.scores.get(name).is_some_and(|s| *s >= prisms.threshold)
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.state.pinned.iter().any(|p| p.id == id)
    }

    /// (all, not seen yet) of one kind.
    pub fn count(&self, kind: Kind) -> (usize, usize) {
        let of_kind = self.state.items.iter().filter(|i| i.kind == kind && self.in_feed(i));
        of_kind.fold((0, 0), |(n, new), i| (n + 1, new + !self.state.seen.contains(&i.id) as usize))
    }

    pub fn is_new(&self, item: &Item) -> bool {
        !self.state.seen.contains(&item.id)
    }

    pub fn current(&self) -> Option<&Item> {
        self.visible().get(self.selected).copied()
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    pub fn toast(&self) -> Option<&str> {
        self.toast.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(3)).map(|(t, _)| t.as_str())
    }

    pub fn busy(&self) -> bool {
        self.busy_mail || self.busy_feeds
    }

    // ── Arrivals ─────────────────────────────────────────────────────────

    pub fn receive(&mut self, msg: Msg) {
        match msg {
            Msg::Busy { mail: true, busy } => self.busy_mail = busy,
            Msg::Busy { mail: false, busy } => {
                self.busy_feeds = busy;
                if !busy {
                    self.forget_dropped_sources();
                }
            }
            Msg::Fetched { origin, result: Err(e) } => {
                self.errors.insert(origin, e);
            }
            Msg::Fetched { origin, result: Ok(items) } => self.arrived(origin, items),
            Msg::MarkedRead { uid, result: Ok(()) } => {
                // Gone from Gmail's unread for good: drop it rather than keep it hidden.
                // Unless it was undone in the meantime: then it stays.
                let id = format!("mail:{uid}");
                if self.state.hidden.contains(&id) {
                    self.say("Marked read in Gmail · ctrl+z undoes");
                    self.state.items.retain(|i| i.id != id);
                    self.reselect();
                }
            }
            Msg::MarkedRead { uid, result: Err(e) } => {
                // Still unread in Gmail, so it comes back here too.
                self.state.hidden.remove(&format!("mail:{uid}"));
                self.say(format!("Gmail did not mark it read: {e}"));
                self.reselect();
            }
            Msg::Scored { id, result: Ok(scores) } => {
                for item in self.state.items.iter_mut().chain(self.state.pinned.iter_mut()).filter(|i| i.id == id) {
                    item.scores = scores.clone();
                }
                self.dirty.get_or_insert_with(Instant::now);
                // A strong candidate that just arrived: that is what the prisms are for.
                let alert = self.prisms.as_ref().map_or(1.1, |p| p.alert);
                let best = scores.iter().filter(|(_, s)| **s >= alert).max_by(|a, b| a.1.total_cmp(b.1));
                if let (Some((name, score)), true) = (best, self.arrivals.remove(&id) && self.config.feed.notify) {
                    if let Some(item) = self.state.items.iter().find(|i| i.id == id) {
                        notify::candidate(item.clone(), name.clone(), *score);
                    }
                }
            }
            Msg::Scored { result: Err(e), .. } => {
                // Once is enough to say it; the rest are the same story.
                if !self.errors.contains_key("jev") {
                    self.say(e.clone());
                }
                self.errors.insert("jev".into(), e);
            }
            Msg::Description { id, result, quiet } => {
                self.loading.remove(&id);
                match result {
                    Ok(text) => {
                        let body = crate::body::text_to_md(&text);
                        let body = if body.trim().is_empty() { "*No description.*".to_string() } else { body };
                        for item in self.state.items.iter_mut().chain(self.state.pinned.iter_mut()).filter(|i| i.id == id) {
                            item.preview = crate::body::plain(&body, 280);
                            item.body = body.clone();
                        }
                        self.bodies.retain(|(key, _), _| key != &id);
                        self.dirty.get_or_insert_with(Instant::now);
                    }
                    Err(_) if quiet => {}
                    Err(e) => {
                        self.expanded.remove(&id);
                        self.say(format!("No description: {e}"));
                    }
                }
            }
        }
    }

    fn arrived(&mut self, origin: String, items: Vec<Item>) {
        self.errors.remove(&origin);
        let fresh: Vec<Item> = items.iter().filter(|i| !self.state.known.contains(&i.id)).cloned().collect();
        let again = !self.synced.insert(origin.clone());
        for item in &fresh {
            self.state.known.insert(item.id.clone());
            if self.first_run && !again {
                self.state.seen.insert(item.id.clone());
            }
        }
        // News is only news once we are up and running: whatever came in while
        // we were closed gets its dot, not a burst of notifications.
        if again {
            self.arrivals.extend(fresh.iter().map(|i| i.id.clone()));
        }
        if again && self.config.feed.notify {
            notify::new_items(fresh);
        }
        // A description or scores found earlier outlive a refresh that came without them.
        let earlier: HashMap<String, &Item> = self.state.items.iter().filter(|i| i.origin == origin).map(|i| (i.id.clone(), i)).collect();
        let mut items = items;
        for item in &mut items {
            let Some(old) = earlier.get(&item.id) else { continue };
            if item.body.is_empty() && !old.body.is_empty() {
                item.body = old.body.clone();
                item.preview = old.preview.clone();
            }
            item.scores = old.scores.clone();
        }
        self.state.items.retain(|i| i.origin != origin);
        self.state.items.extend(items);
        self.sort();
        self.reselect();
        self.score_new();
        self.dirty.get_or_insert_with(Instant::now);
    }

    /// Keep the cursor on the same item when the list changes under it.
    fn reselect(&mut self) {
        let found = self.selected_id.as_ref().and_then(|id| self.visible().iter().position(|i| &i.id == id));
        match found {
            Some(at) => {
                self.selected = at;
                self.snap = true;
            }
            None => self.select(self.selected),
        }
    }

    /// Save now and then, not on every arrival.
    pub fn save_if_due(&mut self, force: bool) {
        if self.dirty.is_some_and(|since| force || since.elapsed() > Duration::from_secs(5)) {
            self.dirty = None;
            let state = store::slim(&self.state, crate::model::now() - self.config.feed.days * 86400);
            self.state.cleared = state.cleared.clone();
            store::save(&state);
        }
    }

    // ── Moving around ────────────────────────────────────────────────────

    pub fn select(&mut self, index: usize) {
        let visible = self.visible();
        let index = index.min(visible.len().saturating_sub(1));
        let id = visible.get(index).map(|i| i.id.clone());
        self.selected = index;
        if let Some(id) = &id {
            if self.state.seen.insert(id.clone()) {
                self.dirty.get_or_insert_with(Instant::now);
            }
        }
        self.selected_id = id;
        self.snap = true;
    }

    fn view_height(&self) -> u16 {
        self.list_area.height.max(1)
    }

    /// Down: through a long item first, then on to the next one.
    pub fn down(&mut self) {
        let h = self.view_height();
        if let Some(span) = self.spans.get(self.selected).copied() {
            if span.bottom > self.scroll + h {
                self.scroll = (self.scroll + h / 2).min(span.bottom.saturating_sub(h));
                return;
            }
        }
        self.select(self.selected + 1);
    }

    pub fn up(&mut self) {
        let h = self.view_height();
        if let Some(span) = self.spans.get(self.selected).copied() {
            if span.top < self.scroll {
                self.scroll = self.scroll.saturating_sub(h / 2).max(span.top);
                return;
            }
        }
        self.select(self.selected.saturating_sub(1));
    }

    pub fn page(&mut self, down: bool) {
        let h = self.view_height().saturating_sub(2).max(1);
        self.scroll = if down { self.scroll + h } else { self.scroll.saturating_sub(h) };
        let target = self.spans.iter().position(|s| s.bottom > self.scroll + 1).unwrap_or(self.spans.len().saturating_sub(1));
        self.select(target);
        self.snap = false;
    }

    /// The wheel moves the page, not the cursor; the cursor only follows
    /// when its item leaves the screen.
    pub fn wheel(&mut self, down: bool) {
        let h = self.view_height();
        let end = self.spans.last().map_or(0, |s| s.bottom).saturating_sub(h);
        self.scroll = if down { (self.scroll + 3).min(end) } else { self.scroll.saturating_sub(3) };
        let Some(span) = self.spans.get(self.selected).copied() else { return };
        if span.bottom <= self.scroll || span.top >= self.scroll + h {
            let target = if down { self.spans.iter().position(|s| s.top >= self.scroll) } else { self.spans.iter().rposition(|s| s.bottom <= self.scroll + h) };
            if let Some(at) = target {
                self.select(at);
                self.snap = false;
            }
        }
    }

    pub fn set_filter(&mut self, filter: Filter) {
        if self.filter != filter {
            self.filter = filter;
            self.scroll = 0;
            self.select(0);
        }
    }

    pub fn cycle_filter(&mut self, back: bool) {
        let at = FILTERS.iter().position(|f| *f == self.filter).unwrap_or(0);
        let next = if back { (at + FILTERS.len() - 1) % FILTERS.len() } else { (at + 1) % FILTERS.len() };
        self.set_filter(FILTERS[next]);
    }

    /// The item under a screen row, if any.
    pub fn item_at(&self, y: u16) -> Option<usize> {
        let area = self.list_area;
        if y < area.y || y >= area.y + area.height {
            return None;
        }
        let row = self.scroll + (y - area.y);
        self.spans.iter().position(|s| row >= s.top && row < s.bottom)
    }

    // ── Doing things ─────────────────────────────────────────────────────

    pub fn toggle(&mut self) {
        let Some(item) = self.current() else { return };
        let (id, url, kind, empty) = (item.id.clone(), item.url.clone(), item.kind, item.body.trim().is_empty());
        if empty {
            if kind != Kind::Video {
                return self.say("Nothing more to show — ⏎ opens it");
            }
            if self.loading.insert(id.clone()) {
                let (tx, id) = (self.tx.clone(), id.clone());
                std::thread::spawn(move || {
                    let result = sources::video_description(&url);
                    let _ = tx.send(Msg::Description { id, result, quiet: false });
                });
            }
        }
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.snap = true;
    }

    // ── Prisms ───────────────────────────────────────────────────────────

    pub fn open_palette(&mut self) {
        match &self.prisms {
            Some(_) => self.palette = Some(Palette { query: String::new(), selected: 0, kind: PaletteKind::Prisms }),
            None => self.say("Prisms need a jev key: TYPESAFE_API_KEY, or [jev] in config.toml"),
        }
    }

    /// The links of the item under the cursor, in a box to pick from.
    pub fn open_links(&mut self) {
        let Some(item) = self.current() else { return };
        let links = crate::ui::links_in(&item.body);
        match links.len() {
            0 => self.say("No links in this one — ⇧⏎ opens the item itself"),
            _ => self.palette = Some(Palette { query: String::new(), selected: 0, kind: PaletteKind::Links(links) }),
        }
    }

    /// What the box offers for what is typed, best match first.
    /// Prisms: in the order they are written. Links: in the order they appear.
    pub fn palette_entries(&self) -> Vec<Entry> {
        let Some(palette) = &self.palette else { return Vec::new() };
        let query = palette.query.to_lowercase();
        match &palette.kind {
            PaletteKind::Prisms => {
                let Some(prisms) = &self.prisms else { return Vec::new() };
                let mut entries: Vec<(u8, usize, Entry)> = prisms
                    .list
                    .iter()
                    .enumerate()
                    .filter_map(|(at, p)| {
                        let rank = match_rank(&p.name.to_lowercase(), &query)?;
                        // How many candidates it holds in the feed right now.
                        let count = self
                            .state
                            .items
                            .iter()
                            .filter(|i| !self.state.hidden.contains(&i.id) && !self.is_pinned(&i.id))
                            .filter(|i| i.scores.get(&p.name).is_some_and(|s| *s >= prisms.threshold))
                            .count();
                        Some((rank, at, Entry { label: p.name.clone(), detail: count.to_string(), value: p.name.clone() }))
                    })
                    .collect();
                entries.sort_by_key(|(rank, at, _)| (*rank, *at));
                entries.into_iter().map(|(_, _, e)| e).collect()
            }
            PaletteKind::Links(links) => {
                let mut entries: Vec<(u8, usize, Entry)> = links
                    .iter()
                    .enumerate()
                    .filter_map(|(at, (text, url))| {
                        let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("").trim_start_matches("www.");
                        let rank = match_rank(&text.to_lowercase(), &query).into_iter().chain(match_rank(&url.to_lowercase(), &query)).min()?;
                        let label = if text.is_empty() { url.clone() } else { text.clone() };
                        Some((rank, at, Entry { label, detail: host.to_string(), value: url.clone() }))
                    })
                    .collect();
                entries.sort_by_key(|(rank, at, _)| (*rank, *at));
                entries.into_iter().map(|(_, _, e)| e).collect()
            }
        }
    }

    /// A key while the box is open.
    pub fn palette_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let count = self.palette_entries().len();
        let Some(palette) = &mut self.palette else { return };
        match key.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Enter => {
                let (selected, links) = (palette.selected, matches!(palette.kind, PaletteKind::Links(_)));
                let picked = self.palette_entries().into_iter().nth(selected).map(|e| e.value);
                self.palette = None;
                match picked {
                    Some(url) if links => self.open_url(&url),
                    Some(name) => self.set_prism(Some(name)),
                    None => {}
                }
            }
            KeyCode::Down | KeyCode::Tab => palette.selected = (palette.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Char('n') if ctrl => palette.selected = (palette.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Up | KeyCode::BackTab => palette.selected = palette.selected.saturating_sub(1),
            KeyCode::Char('p') if ctrl => palette.selected = palette.selected.saturating_sub(1),
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            KeyCode::Char('u') if ctrl => {
                palette.query.clear();
                palette.selected = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                palette.query.push(c);
                palette.selected = 0;
            }
            _ => {}
        }
    }

    pub fn set_prism(&mut self, prism: Option<String>) {
        self.prism = prism;
        self.scroll = 0;
        self.select(0);
    }

    pub fn collapse(&mut self) {
        if let Some(id) = self.current().map(|i| i.id.clone()) {
            if self.expanded.remove(&id) {
                self.snap = true;
            }
        }
    }

    pub fn open(&mut self) {
        let Some(url) = self.current().map(|i| i.url.clone()) else { return };
        self.open_url(&url);
    }

    pub fn open_url(&mut self, url: &str) {
        match notify::spawn(notify::browser(), &[url]) {
            Ok(()) => self.say("Opened in the browser"),
            Err(e) => self.say(format!("Could not open it: {e}")),
        }
    }

    /// The link drawn at this spot on screen, if there is one.
    pub fn link_at(&self, x: u16, y: u16) -> Option<String> {
        self.links.iter().find(|(row, x0, x1, _)| *row == y && (*x0..*x1).contains(&x)).map(|(.., url)| url.clone())
    }

    pub fn play(&mut self) {
        let Some(item) = self.current() else { return };
        if item.kind != Kind::Video {
            return self.say("p plays videos; ⏎ opens this in the browser");
        }
        let url = item.url.clone();
        match notify::spawn("mpv", &[&url]) {
            Ok(()) => self.say("Playing in mpv…"),
            Err(e) => self.say(format!("Could not start mpv: {e}")),
        }
    }

    /// Into the omanote inbox, as a line with the link.
    pub fn save(&mut self) {
        let Some(item) = self.current() else { return };
        let text = format!("{} — {} {}", item.source, item.title, item.url);
        let out = std::process::Command::new("omanote").args(["--capture", &text]).output();
        match out {
            Ok(o) if o.status.success() => self.say("Saved to the omanote inbox"),
            Ok(o) => self.say(format!("omanote: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Err(_) => self.say("omanote is not installed"),
        }
    }

    /// Into the Pins tab and out of the feed, or back again.
    pub fn pin(&mut self) {
        let Some(item) = self.current().cloned() else { return };
        if self.is_pinned(&item.id) {
            self.state.pinned.retain(|p| p.id != item.id);
            self.say("Unpinned");
        } else {
            self.state.seen.insert(item.id.clone());
            self.state.pinned.insert(0, item);
            self.say("Pinned · 5 or ⇥ shows your pins");
        }
        self.dirty.get_or_insert_with(Instant::now);
        self.select(self.selected);
    }

    /// Mail is marked read in Gmail; anything else is cleared from the feed.
    /// A pinned item is unpinned too: done is done.
    pub fn done(&mut self) {
        let Some(item) = self.current().cloned() else { return };
        let (id, uid, kind) = (item.id.clone(), item.uid, item.kind);
        let pinned = self.is_pinned(&id);
        self.undo.push((item, pinned));
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.state.pinned.retain(|p| p.id != id);
        if let (Kind::Mail, Some(uid), Some(gmail)) = (kind, uid, &self.gmail) {
            let _ = gmail.send(Cmd::MarkRead(uid));
            self.say("Marking read in Gmail…");
        } else {
            self.say("Cleared · ctrl+z undoes");
        }
        self.state.hidden.insert(id);
        self.dirty.get_or_insert_with(Instant::now);
        self.select(self.selected);
    }

    /// Bring back what `enter` last cleared: into the feed (and its pin), and
    /// for a mail, unread again in Gmail. The cursor goes to it.
    pub fn undo(&mut self) {
        let Some((item, pinned)) = self.undo.pop() else { return self.say("Nothing to undo") };
        let id = item.id.clone();
        self.state.hidden.remove(&id);
        self.state.cleared.remove(&id);
        if !self.state.items.iter().any(|i| i.id == id) {
            self.state.items.push(item.clone());
            self.sort();
        }
        if pinned && !self.is_pinned(&id) {
            self.state.pinned.insert(0, item.clone());
        }
        match (item.kind, item.uid, &self.gmail) {
            (Kind::Mail, Some(uid), Some(gmail)) => {
                let _ = gmail.send(Cmd::MarkUnread(uid));
                self.say(format!("Back, and unread again in Gmail: {}", item.title));
            }
            _ => self.say(format!("Back: {}", item.title)),
        }
        self.dirty.get_or_insert_with(Instant::now);
        // Where it is shown: its own tab if pinned, otherwise the feed.
        if pinned {
            self.filter = Filter::Pinned;
        } else if self.filter == Filter::Pinned {
            self.filter = Filter::All;
        }
        if !self.in_prism(&item) {
            self.prism = None;
        }
        self.selected_id = Some(id);
        self.reselect();
    }

    pub fn refresh(&mut self) {
        for worker in [&self.gmail, &self.feeds].into_iter().flatten() {
            let _ = worker.send(Cmd::Refresh);
        }
        self.say("Refreshing…");
    }
}

/// How well `name` matches what is typed: 0 starts with it, 1 has a word
/// starting with it, 2 contains it, 3 has its letters in order. None: no match.
fn match_rank(name: &str, query: &str) -> Option<u8> {
    if query.is_empty() || name.starts_with(query) {
        return Some(0);
    }
    if name.split_whitespace().any(|w| w.starts_with(query)) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    let mut letters = name.chars();
    query.chars().filter(|c| !c.is_whitespace()).all(|q| letters.any(|c| c == q)).then_some(3)
}

#[cfg(test)]
mod tests {
    use super::match_rank;

    #[test]
    fn matches_rank_prefixes_first() {
        assert_eq!(match_rank("israel news", "is"), Some(0));
        assert_eq!(match_rank("world news", "ne"), Some(1));
        assert_eq!(match_rank("politics", "tic"), Some(2));
        assert_eq!(match_rank("bay area news", "ban"), Some(3));
        assert_eq!(match_rank("ai", "x"), None);
        assert_eq!(match_rank("ai", ""), Some(0));
    }
}
