//! Drawing. One centred column, like a timeline: a quiet header with the
//! filters, the items separated by hairlines, a line of key hints at the foot.
//!
//! Colours come from the terminal's own palette, so the feed wears whatever
//! Omarchy theme is on; the few derived tones (the selected item's surface,
//! muted text) are mixed from the terminal's real background and foreground.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span as TSpan};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, FILTERS, Filter, Span};
use crate::layout::{VRow, layout};
use crate::markdown::{Block, LinkTo, classify, link_at};
use crate::model::{Item, Kind, age, count, now};
use crate::table::{self, Role};
use crate::theme;

/// Widest the column gets, so lines stay readable on a wide screen.
const MAX_WIDTH: u16 = 84;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn tint(kind: Kind) -> Color {
    match kind {
        Kind::Mail => Color::Blue,
        Kind::Video => Color::Red,
        Kind::Post => Color::Yellow,
    }
}

fn muted() -> Style {
    theme::get().muted()
}

fn faint() -> Style {
    theme::get().faint()
}

/// A link on a rendered row: the columns it covers, where it goes, what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub x0: u16,
    pub x1: u16,
    pub url: String,
    pub text: String,
}

/// One row of an item as laid out, before it is placed on screen.
enum Row {
    Text(Line<'static>),
    /// A row of a body, with the links on it.
    Body(Line<'static>, Vec<Hit>),
    /// Row `r` of a picture `cols` × `rows` cells.
    Image { url: String, cols: u16, rows: u16, r: u16 },
    Blank,
    /// The hairline after an item: outside its highlight.
    Rule,
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let buf = f.buffer_mut();
    if area.width < 20 || area.height < 8 {
        buf.set_string(area.x, area.y, "omafeed: too small", muted());
        return;
    }
    let width = area.width.saturating_sub(2).min(MAX_WIDTH);
    let col = Rect { x: area.x + (area.width - width) / 2, y: area.y, width, height: area.height };

    header(buf, app, Rect { height: 1, y: col.y + 1, ..col });
    buf.set_string(col.x, col.y + 2, "─".repeat(col.width as usize), faint());
    // The prism looked through sits in the rule under the header.
    if let Some(prism) = &app.prism {
        let chip = Line::from(vec![
            TSpan::raw(" "),
            TSpan::styled(format!("󰌵 {prism}"), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            TSpan::styled("  esc clears ", muted()),
        ]);
        buf.set_line(col.x + 2, col.y + 2, &chip, col.width.saturating_sub(4));
    }
    let list = Rect { y: col.y + 3, height: col.height.saturating_sub(5), ..col };
    footer(buf, app, Rect { y: col.y + col.height - 1, height: 1, ..col });
    app.list_area = list;
    feed(buf, app, list);
    if app.palette.is_some() {
        palette(buf, app, list);
    }
}

/// The box over the feed (prisms, or an item's links): what is typed, and what matches it.
fn palette(buf: &mut Buffer, app: &App, list: Rect) {
    let Some(state) = &app.palette else { return };
    let entries = app.palette_entries();
    let shown = entries.len().clamp(1, 10) as u16;
    let width = list.width.saturating_sub(8).clamp(20, 52);
    let area = Rect { x: list.x + (list.width - width) / 2, y: list.y + 1, width, height: shown + 4 };
    let look = theme::get();
    let fill = look.raised().remove_modifier(Modifier::BOLD).remove_modifier(Modifier::REVERSED);
    let fill = if look.rich { fill } else { Style::new() };
    buf.set_style(area, fill);
    for y in area.y..area.y + area.height {
        buf.set_string(area.x, y, " ".repeat(area.width as usize), fill);
    }
    let edge = faint().patch(fill);
    let inner = area.width.saturating_sub(2) as usize;
    buf.set_string(area.x, area.y, format!("╭{}╮", "─".repeat(inner)), edge);
    buf.set_string(area.x, area.y + area.height - 1, format!("╰{}╯", "─".repeat(inner)), edge);
    for y in area.y + 1..area.y + area.height - 1 {
        buf.set_string(area.x, y, "│", edge);
        buf.set_string(area.x + area.width - 1, y, "│", edge);
    }
    let title = match state.kind {
        crate::app::PaletteKind::Prisms => " Prisms ",
        crate::app::PaletteKind::Links(_) => " Links ",
    };
    buf.set_string(area.x + 2, area.y, title, Style::new().add_modifier(Modifier::BOLD).patch(fill));

    let x = area.x + 2;
    let w = area.width.saturating_sub(4);
    let query = Line::from(vec![
        TSpan::styled("󰍉 ", Style::new().fg(Color::Cyan)),
        TSpan::raw(state.query.clone()),
        TSpan::styled("▏", Style::new().fg(Color::Cyan)),
        TSpan::styled(if state.query.is_empty() { "type to filter" } else { "" }, muted()),
    ]);
    buf.set_line(x, area.y + 1, &query, w);
    buf.set_string(x, area.y + 2, "─".repeat(w as usize), edge);

    if entries.is_empty() {
        buf.set_string(x, area.y + 3, "Nothing matches", muted().patch(fill));
        return;
    }
    // Keep the pick on screen when the list is longer than the box.
    let first = state.selected.saturating_sub(shown as usize - 1);
    for (k, entry) in entries.iter().enumerate().skip(first).take(shown as usize) {
        let y = area.y + 3 + (k - first) as u16;
        let picked = k == state.selected;
        let row = if picked { look.raised() } else { fill };
        buf.set_string(area.x + 1, y, " ".repeat(inner), row);
        let detail = truncate(&entry.detail, (w as usize / 3).max(4));
        let dw = detail.width() as u16;
        let label = truncate(&entry.label, (w as usize).saturating_sub(dw as usize + 4));
        buf.set_string(x, y, if picked { "› " } else { "  " }, Style::new().fg(Color::Cyan).patch(row));
        buf.set_string(x + 2, y, &label, row);
        buf.set_string(x + w - dw, y, &detail, muted().patch(row));
    }
}

// ── Header and footer ────────────────────────────────────────────────────

fn header(buf: &mut Buffer, app: &App, row: Rect) {
    let x = row.x + 1;
    let mut spans = vec![TSpan::styled("omafeed", Style::new().add_modifier(Modifier::BOLD))];
    spans.push(TSpan::raw("   "));
    for filter in FILTERS {
        let active = app.filter == filter;
        let (label, new, tint) = match filter {
            Filter::All => ("All".to_string(), 0, None),
            Filter::Only(kind) => {
                let (n, new) = app.count(kind);
                (format!("{} {n}", kind.icon()), new, Some(self::tint(kind)))
            }
            Filter::Pinned => (format!("󰐃 {}", app.state.pinned.len()), 0, None),
        };
        let style = match (active, new > 0, tint) {
            (true, _, _) => theme::get().raised(),
            (false, true, Some(t)) => Style::new().fg(t),
            _ => muted(),
        };
        spans.push(TSpan::styled(format!(" {label} "), style));
        spans.push(TSpan::raw(" "));
    }
    let line = Line::from(spans);
    buf.set_line(x, row.y, &line, row.width.saturating_sub(2));

    let status: Line = if app.busy() {
        let frame = SPINNER[(app.tick / 2) as usize % SPINNER.len()];
        Line::from(vec![TSpan::styled(frame, Style::new().fg(Color::Cyan)), TSpan::styled(" syncing", muted())])
    } else if app.errors.contains_key(crate::sources::GMAIL) {
        Line::from(vec![TSpan::styled("●", Style::new().fg(Color::Red)), TSpan::styled(" offline", muted())])
    } else {
        Line::from(vec![TSpan::styled("●", Style::new().fg(Color::Green)), TSpan::styled(" live", muted())])
    };
    let w = status.width() as u16;
    if line.width() as u16 + w + 4 <= row.width {
        buf.set_line(row.x + row.width - w - 1, row.y, &status, w);
    }
}

fn footer(buf: &mut Buffer, app: &App, row: Rect) {
    let line = if let Some(toast) = app.toast() {
        Line::from(vec![TSpan::styled("› ", Style::new().fg(Color::Cyan)), TSpan::raw(toast.to_string())])
    } else if let Some(e) = app.errors.get(crate::sources::GMAIL) {
        Line::from(vec![TSpan::styled("󰇮 ", Style::new().fg(Color::Red)), TSpan::styled(e.clone(), muted())])
    } else {
        let keys: &[(&str, &str)] = &[("␣", "expand"), ("⏎", "done"), ("⇧⏎", "open"), ("b", "pin"), ("^P", "prisms"), ("l", "links"), ("s", "save"), ("⇥", "filter"), ("r", "refresh"), ("q", "quit")];
        let mut spans = Vec::new();
        let mut used = 0;
        for (key, what) in keys {
            let w = key.width() + what.width() + 3;
            if used + w > row.width.saturating_sub(2) as usize {
                break;
            }
            used += w;
            spans.push(TSpan::styled(*key, Style::new().add_modifier(Modifier::BOLD)));
            spans.push(TSpan::styled(format!(" {what}   "), muted()));
        }
        let failed = app.errors.len() - app.errors.contains_key(crate::sources::GMAIL) as usize;
        if failed > 0 && used + 12 < row.width as usize {
            spans.push(TSpan::styled(format!("{failed} feed{} failed", if failed == 1 { "" } else { "s" }), faint()));
        }
        Line::from(spans)
    };
    buf.set_line(row.x + 1, row.y, &line, row.width.saturating_sub(2));
}

// ── The feed ─────────────────────────────────────────────────────────────

fn feed(buf: &mut Buffer, app: &mut App, area: Rect) {
    app.links.clear();
    app.thumbs.begin_frame();
    app.tick += 1;
    let now = now();
    // Content sits inside a gutter: the selection bar on the left.
    let inner = area.width.saturating_sub(6);
    let items: Vec<Item> = app.visible().into_iter().cloned().collect();
    if items.is_empty() {
        app.spans.clear();
        return empty(buf, app, area);
    }

    let mut blocks: Vec<Vec<Row>> = Vec::with_capacity(items.len());
    let mut spans = Vec::with_capacity(items.len());
    let mut top = 0u16;
    for item in &items {
        let expanded = app.expanded.contains(&item.id);
        let rows = item_rows(app, item, inner, expanded, now);
        let height = rows.len() as u16;
        spans.push(Span { top, bottom: top + height });
        top = top.saturating_add(height);
        blocks.push(rows);
    }
    app.spans = spans;

    // Scroll: keep the selected item in view when asked to, never past the end.
    let h = area.height;
    if app.snap {
        app.snap = false;
        if let Some(s) = app.spans.get(app.selected) {
            if s.top < app.scroll || s.bottom - s.top > h {
                app.scroll = s.top;
            } else if s.bottom > app.scroll + h {
                app.scroll = s.bottom - h;
            }
        }
    }
    app.scroll = app.scroll.min(top.saturating_sub(h));

    let surface = theme::get().surface();
    for (index, rows) in blocks.into_iter().enumerate() {
        let span = app.spans[index];
        if span.bottom <= app.scroll || span.top >= app.scroll + h {
            continue;
        }
        let selected = index == app.selected;
        for (k, row) in rows.into_iter().enumerate() {
            let at = span.top + k as u16;
            if at < app.scroll || at >= app.scroll + h {
                continue;
            }
            let y = area.y + at - app.scroll;
            let is_rule = matches!(row, Row::Rule);
            // Selection is the surface alone: nothing moves, nothing is added.
            if selected && !is_rule {
                buf.set_style(Rect { x: area.x, y, width: area.width, height: 1 }, surface);
            }
            let x = area.x + 3;
            match row {
                Row::Text(line) => {
                    buf.set_line(x, y, &line, inner);
                }
                Row::Body(line, hits) => {
                    buf.set_line(x, y, &line, inner);
                    // Remembered for clicks: this frame's links, where they are on screen.
                    app.links.extend(hits.into_iter().map(|h| (y, x + h.x0, x + h.x1, h.url)));
                }
                Row::Blank => {}
                Row::Rule => {
                    buf.set_string(area.x + 1, y, "─".repeat(area.width.saturating_sub(2) as usize), faint());
                }
                Row::Image { url, cols, rows, r } => match app.thumbs.get(&url, cols, rows) {
                    Some(picture) => {
                        let mut cx = x;
                        for (text, style) in &picture[r as usize] {
                            let (next, _) = buf.set_stringn(cx, y, text, (x + cols - cx) as usize, *style);
                            cx = next;
                        }
                    }
                    None => skeleton(buf, x, y, cols, rows, r),
                },
            }
        }
    }
}

/// A picture that is still on its way: a quiet box of the right size.
fn skeleton(buf: &mut Buffer, x: u16, y: u16, cols: u16, rows: u16, r: u16) {
    let tone = theme::get().surface().bg.unwrap_or(Color::DarkGray);
    let tone = if tone == Color::Reset { Color::DarkGray } else { tone };
    let shade = match theme::get().rich {
        true => Style::new().bg(tone).fg(muted().fg.unwrap_or(Color::DarkGray)),
        false => Style::new().fg(Color::DarkGray),
    };
    let fill = if theme::get().rich { " " } else { "░" };
    buf.set_string(x, y, fill.repeat(cols as usize), shade);
    if r == rows / 2 {
        buf.set_string(x + cols / 2, y, "󰗃", shade);
    }
}

fn empty(buf: &mut Buffer, app: &App, area: Rect) {
    let lines: Vec<Line> = if !app.has_sources() {
        let dir = crate::config::dir().display().to_string().replace(&std::env::var("HOME").unwrap_or_default(), "~");
        vec![
            Line::styled("󰑫", Style::new().fg(Color::Yellow)),
            Line::raw(""),
            Line::styled("Nothing to follow yet", Style::new().add_modifier(Modifier::BOLD)),
            Line::raw(""),
            Line::styled(format!("Feeds go in {dir}/feeds"), muted()),
            Line::styled(format!("Gmail in {dir}/config.toml"), muted()),
            Line::raw(""),
            Line::styled("omafeed --import-youtube subscriptions.csv", faint()),
        ]
    } else if app.state.items.is_empty() && !app.busy() && !app.errors.is_empty() {
        let n = app.errors.len();
        let first = app.errors.values().next().cloned().unwrap_or_default();
        vec![
            Line::styled("󰅛", Style::new().fg(Color::Red)),
            Line::raw(""),
            Line::styled(format!("Could not load {n} source{}", if n == 1 { "" } else { "s" }), Style::new().add_modifier(Modifier::BOLD)),
            Line::raw(""),
            Line::styled(truncate(&first, area.width.saturating_sub(4) as usize), muted()),
            Line::raw(""),
            Line::styled("Check ~/.config/omafeed/feeds, then r to retry", faint()),
        ]
    } else if app.busy() && app.state.items.is_empty() {
        vec![Line::styled(SPINNER[(app.tick / 2) as usize % SPINNER.len()], Style::new().fg(Color::Cyan)), Line::raw(""), Line::styled("Fetching your feed…", muted())]
    } else {
        let days = app.config.feed.days;
        let what = match app.filter {
            Filter::All => "Nothing new",
            Filter::Only(Kind::Mail) => "Inbox zero",
            Filter::Only(Kind::Video) => "No new videos",
            Filter::Only(Kind::Post) => "No new posts",
            Filter::Pinned => "No pins yet",
        };
        let note = match app.filter {
            Filter::Pinned => "b on anything keeps it here".to_string(),
            _ => format!("You are all caught up for the last {days} days"),
        };
        vec![
            Line::styled("󰄬", Style::new().fg(Color::Green)),
            Line::raw(""),
            Line::styled(what, Style::new().add_modifier(Modifier::BOLD)),
            Line::styled(note, muted()),
        ]
    };
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    for (k, line) in lines.iter().enumerate() {
        let w = (line.width() as u16).min(area.width);
        buf.set_line(area.x + (area.width - w) / 2, top.saturating_sub(2) + k as u16, line, w);
    }
}

// ── One item ─────────────────────────────────────────────────────────────

fn item_rows(app: &mut App, item: &Item, width: u16, expanded: bool, now: i64) -> Vec<Row> {
    // The site's own icon, two cells wide, once it has loaded; the glyph until then.
    let icon = item.icon.as_deref().filter(|_| app.thumbs.kitty()).and_then(|url| app.thumbs.get(url, 2, 1)).and_then(|rows| rows.first().cloned());
    // The prism it passes (the one looked through, else its strongest), and how many more.
    let hits = app.matches(item);
    let tag = match &app.prism {
        Some(name) => item.scores.get(name).map(|s| (name.clone(), *s, hits.len().saturating_sub(1))),
        None => hits.first().map(|(name, s)| (name.clone(), *s, hits.len() - 1)),
    };
    let mut rows = vec![Row::Blank, Row::Text(head(item, width, app.is_new(item), now, icon, tag))];
    let title = Style::new().add_modifier(Modifier::BOLD);
    let preview_lines = 2;
    match item.kind {
        Kind::Mail => {
            rows.extend(wrap(&item.title, width, if expanded { 99 } else { 2 }, title));
            if !expanded {
                rows.extend(wrap(&item.preview, width, preview_lines, muted()));
            }
        }
        Kind::Video => {
            rows.extend(picture(app, item, width));
            rows.extend(wrap(&item.title, width, if expanded { 99 } else { 2 }, title));
        }
        Kind::Post => {
            rows.extend(wrap(&item.title, width, if expanded { 99 } else { 2 }, title));
            rows.extend(picture(app, item, width));
            if !expanded {
                rows.extend(wrap(&item.preview, width, preview_lines, muted()));
            }
        }
    }
    if expanded && app.loading.contains(&item.id) {
        rows.push(Row::Blank);
        let frame = SPINNER[(app.tick / 2) as usize % SPINNER.len()];
        rows.push(Row::Text(Line::from(vec![TSpan::styled(frame, Style::new().fg(tint(item.kind))), TSpan::styled(" Loading the description…", muted())])));
    } else if expanded {
        rows.push(Row::Blank);
        let key = (item.id.clone(), width);
        if !app.bodies.contains_key(&key) {
            app.bodies.insert(key.clone(), markdown(&item.body, width));
        }
        rows.extend(app.bodies[&key].iter().cloned().map(|(line, hits)| Row::Body(line, hits)));
    }
    rows.push(Row::Blank);
    rows.push(Row::Rule);
    rows
}

fn picture(app: &App, item: &Item, width: u16) -> Vec<Row> {
    let Some(url) = item.thumb.clone().filter(|_| app.thumbs.enabled()) else { return Vec::new() };
    // As wide as the text, so the picture lines up with everything else.
    let cols = width;
    let rows = app.thumbs.rows_for(cols);
    let mut out: Vec<Row> = (0..rows).map(|r| Row::Image { url: url.clone(), cols, rows, r }).collect();
    out.insert(0, Row::Blank);
    out.push(Row::Blank);
    out
}

/// `󰇮  Jane Doe                              ● 12m`
fn head(item: &Item, width: u16, new: bool, now: i64, icon: Option<crate::thumbs::Row>, tag: Option<(String, f32, usize)>) -> Line<'static> {
    let mut right = Vec::new();
    if let Some((name, score, more)) = tag {
        let more = if more > 0 { format!(" +{more}") } else { String::new() };
        right.push(TSpan::styled(format!("󰌵 {name} {:.0}%{more}", score * 100.0), Style::new().fg(Color::Cyan)));
        right.push(TSpan::styled(" · ", muted()));
    }
    if let Some(v) = item.views {
        right.push(TSpan::styled(format!("{} views · ", count(v)), muted()));
    }
    right.push(TSpan::styled(age(item.time, now), muted()));
    if new {
        right.push(TSpan::styled(" ●", Style::new().fg(tint(item.kind))));
    }
    let right_w: usize = right.iter().map(|s| s.width()).sum();
    // Either way three cells, so the names line up: glyph and two spaces, or icon and one.
    let mut spans: Vec<TSpan<'static>> = match icon {
        Some(picture) => picture.into_iter().map(|(text, style)| TSpan::styled(text, style)).chain([TSpan::raw(" ")]).collect(),
        None => vec![TSpan::styled(format!("{}  ", item.kind.icon()), Style::new().fg(tint(item.kind)))],
    };
    let icon_w = 3;
    let room = (width as usize).saturating_sub(right_w + icon_w + 2);
    let source = TSpan::styled(truncate(&item.source, room), Style::new().add_modifier(Modifier::BOLD));
    let gap = (width as usize).saturating_sub(icon_w + source.width() + right_w);
    spans.push(source);
    spans.push(TSpan::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}

fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in text.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Word-wrap to `width`, at most `max` rows, the last one ending in `…` if cut.
fn wrap(text: &str, width: u16, max: usize, style: Style) -> Vec<Row> {
    let width = width.max(8) as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    let mut cut = false;
    'words: for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let ww = word.width();
            let sep = if cur.is_empty() { 0 } else { 1 };
            if cur_w + sep + ww <= width {
                if sep == 1 {
                    cur.push(' ');
                }
                cur.push_str(&word);
                cur_w += sep + ww;
                break;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
                if lines.len() == max {
                    cut = true;
                    break 'words;
                }
                continue;
            }
            // One word longer than the line: break it.
            let head = truncate(&word, width + 1);
            let head = head.trim_end_matches('…').to_string();
            word = word[head.len()..].to_string();
            lines.push(head);
            if lines.len() == max {
                cut = true;
                break 'words;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max {
        lines.truncate(max);
        cut = true;
    }
    if let Some(last) = lines.last_mut().filter(|_| cut) {
        let fitted = truncate(last, width - 1);
        *last = if fitted.ends_with('…') { fitted } else { format!("{fitted}…") };
    }
    lines.into_iter().map(|l| Row::Text(Line::styled(l, style))).collect()
}

/// A body, drawn by omanote's markdown renderer, with where its links landed.
pub fn markdown(md: &str, width: u16) -> Vec<(Line<'static>, Vec<Hit>)> {
    let lines: Vec<Vec<char>> = md.lines().map(|l| l.chars().collect()).collect();
    let blocks = classify(&lines);
    let mut out = Vec::new();
    for (i, chars) in lines.iter().enumerate() {
        let rows = match blocks[i] {
            Block::Table { start, end } => table::layout_row(chars, &table_ctx(&lines, i, start, end, width), false),
            // Fence lines are syntax; the code gutter shows where code is.
            Block::FenceOpen | Block::FenceClose => continue,
            block => layout(chars, block, false, width),
        };
        out.extend(rows.into_iter().map(|row| {
            let hits = hits(chars, &row);
            (vrow_line(row), hits)
        }));
    }
    while out.last().is_some_and(|(l, _): &(Line, _)| l.width() == 0) {
        out.pop();
    }
    out
}

/// The links on one visual row. Link text is underlined, so only those cells
/// are looked up; each maps back to the source column it was drawn from.
fn hits(chars: &[char], row: &VRow) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    let mut x = row.lead_w;
    for cell in &row.cells {
        let underlined = cell.style.add_modifier.contains(Modifier::UNDERLINED);
        let url = underlined.then(|| link_at(chars, cell.col)).flatten().and_then(|to| match to {
            LinkTo::Target(url) if url.starts_with("http://") || url.starts_with("https://") => Some(url),
            _ => None,
        });
        if let Some(url) = url {
            match out.last_mut() {
                Some(hit) if hit.url == url && hit.x1 == x => {
                    hit.x1 += cell.width;
                    hit.text.push_str(&cell.text);
                }
                _ => out.push(Hit { x0: x, x1: x + cell.width, url, text: cell.text.clone() }),
            }
        }
        x += cell.width;
    }
    out
}

/// Every link in a body, in order, once each: (what it says, where it goes).
pub fn links_in(md: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for hit in markdown(md, 400).into_iter().flat_map(|(_, hits)| hits) {
        match out.iter_mut().find(|(_, url)| *url == hit.url) {
            Some((text, _)) if !text.contains(hit.text.trim()) => {
                text.push(' ');
                text.push_str(hit.text.trim());
            }
            Some(_) => {}
            None => out.push((hit.text.trim().to_string(), hit.url)),
        }
    }
    out
}

fn table_ctx(lines: &[Vec<char>], row: usize, start: usize, end: usize, room: u16) -> table::Ctx {
    let role = |r: usize| match r - start {
        0 => Role::Header,
        1 => Role::Separator,
        _ => Role::Body,
    };
    let mut widths: Vec<u16> = Vec::new();
    for r in start..end {
        for (k, w) in table::measure(&lines[r], role(r), false).into_iter().enumerate() {
            if k == widths.len() {
                widths.push(w);
            }
            widths[k] = widths[k].max(w);
        }
    }
    let widths = table::fit(&widths, room.saturating_sub(widths.len() as u16 + 1));
    table::Ctx { widths, aligns: table::aligns(&lines[start + 1]), role: role(row), top: row == start, bottom: row + 1 == end, divider: row > start + 2 }
}

fn vrow_line(row: VRow) -> Line<'static> {
    let mut spans: Vec<TSpan<'static>> = row.lead.into_iter().map(|(t, s)| TSpan::styled(t, s)).collect();
    // Neighbouring cells of one style become one span.
    for cell in row.cells {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => last.content.to_mut().push_str(&cell.text),
            _ => spans.push(TSpan::styled(cell.text, cell.style)),
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(rows: Vec<Row>) -> Vec<String> {
        rows.into_iter()
            .map(|r| match r {
                Row::Text(l) => l.spans.iter().map(|s| s.content.to_string()).collect(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn wraps_and_cuts_with_an_ellipsis() {
        assert_eq!(texts(wrap("alpha beta gamma delta", 11, 5, Style::new())), ["alpha beta", "gamma delta"]);
        let cut = texts(wrap("alpha beta gamma delta epsilon", 11, 2, Style::new()));
        assert_eq!(cut.len(), 2);
        assert!(cut[1].ends_with('…') && cut[1].width() <= 11, "{cut:?}");
        assert_eq!(texts(wrap("", 10, 2, Style::new())).len(), 0);
        let long = texts(wrap("https://example.com/a/very/long/path/that/goes/on", 10, 3, Style::new()));
        assert!(long.iter().all(|l| l.width() <= 10), "{long:?}");
    }

    #[test]
    fn markdown_bodies_render_without_syntax() {
        let lines = markdown("# Hello\n\nSome **bold** and a [link](https://x.y).\n\n- one", 40);
        let text: Vec<String> = lines.iter().map(|(l, _)| l.spans.iter().map(|s| s.content.to_string()).collect()).collect();
        assert_eq!(text, ["Hello", "", "Some bold and a link.", "", "• one"]);
    }

    #[test]
    fn links_are_found_where_they_are_drawn() {
        let lines = markdown("Read [the post](https://x.y/a) or https://b.c/d now", 80);
        let hits = &lines[0].1;
        assert_eq!(hits.len(), 2);
        // "Read the post or https://b.c/d now": the text sits at 5..13, the bare address at 17..30.
        assert_eq!((hits[0].x0, hits[0].x1, hits[0].url.as_str(), hits[0].text.as_str()), (5, 13, "https://x.y/a", "the post"));
        assert_eq!((hits[1].x0, hits[1].url.as_str()), (17, "https://b.c/d"));

        // A link wrapped over two rows is one link in the list.
        let wrapped = markdown("[a long link text that wraps](https://x.y/w)", 12);
        assert!(wrapped.len() > 1 && wrapped.iter().all(|(_, h)| h.iter().all(|h| h.url == "https://x.y/w")));
        let list = links_in("[a long link text that wraps](https://x.y/w) and [again](https://x.y/w) and [two](https://z.z)");
        assert_eq!(list.len(), 2);
        assert_eq!(list[1], ("two".to_string(), "https://z.z".to_string()));
        assert!(links_in("no links, **bold** only").is_empty());
    }
}

