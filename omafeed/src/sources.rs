//! Background workers. Each source runs on its own thread, fetches on a timer
//! (or when asked), and sends whole results back to the UI thread.

use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mail_parser::{MessageParser, PartType};

use crate::body::{html_to_md, plain, text_to_md};
use crate::config::{self, Feed, Gmail};
use crate::imap::{Imap, search_results};
use crate::model::{Item, Kind, now};

pub const GMAIL: &str = "gmail";
const PREVIEW: usize = 280;
/// A mail's first bytes: enough for its text, not its attachments.
const MAIL_BYTES: usize = 200_000;
const FEED_BYTES: u64 = 10 * 1024 * 1024;
const FEED_THREADS: usize = 8;

pub enum Msg {
    Fetched { origin: String, result: Result<Vec<Item>, String> },
    Busy { mail: bool, busy: bool },
    /// `quiet`: fetched in the background, so a failure is not news.
    Description { id: String, result: Result<String, String>, quiet: bool },
    /// jev's answers for an item, one per prism.
    Scored { id: String, result: Result<std::collections::BTreeMap<String, f32>, String> },
    /// Gmail's answer to marking a mail read.
    MarkedRead { uid: u32, result: Result<(), String> },
}

pub enum Cmd {
    Refresh,
    MarkRead(u32),
    /// Undo of a mark-read: unread again, then a sync so it is back in the feed.
    MarkUnread(u32),
}

/// Keep a worker going until the app hangs up: `work` on every tick or command.
fn worker<S: Send + 'static>(every: Duration, mut state: S, mut work: impl FnMut(&mut S, Option<Cmd>) + Send + 'static) -> Sender<Cmd> {
    let (tx, rx): (Sender<Cmd>, Receiver<Cmd>) = channel();
    std::thread::spawn(move || {
        work(&mut state, None);
        loop {
            match rx.recv_timeout(every) {
                Ok(cmd) => work(&mut state, Some(cmd)),
                Err(RecvTimeoutError::Timeout) => work(&mut state, None),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    tx
}

// ── Gmail ────────────────────────────────────────────────────────────────

pub fn gmail(cfg: Gmail, tx: Sender<Msg>) -> Sender<Cmd> {
    let every = Duration::from_secs(cfg.poll_seconds.max(15));
    worker(every, HashMap::<u32, Item>::new(), move |cache, cmd| {
        let _ = tx.send(Msg::Busy { mail: true, busy: true });
        match cmd {
            Some(Cmd::MarkRead(uid)) => {
                let result = cfg.password().and_then(|pw| mark_read(&cfg, &pw, uid));
                if result.is_ok() {
                    cache.remove(&uid);
                }
                let _ = tx.send(Msg::Busy { mail: true, busy: false });
                let _ = tx.send(Msg::MarkedRead { uid, result });
            }
            Some(Cmd::MarkUnread(uid)) => {
                let result = cfg.password().and_then(|pw| mark_unread(&cfg, &pw, uid).and_then(|_| sync_mail(&cfg, &pw, cache)));
                let _ = tx.send(Msg::Busy { mail: true, busy: false });
                let _ = tx.send(Msg::Fetched { origin: GMAIL.into(), result });
            }
            _ => {
                let result = cfg.password().and_then(|pw| sync_mail(&cfg, &pw, cache));
                let _ = tx.send(Msg::Busy { mail: true, busy: false });
                let _ = tx.send(Msg::Fetched { origin: GMAIL.into(), result });
            }
        }
    })
}

fn session(cfg: &Gmail, pw: &str) -> Result<Imap, String> {
    let mut imap = Imap::connect("imap.gmail.com")?;
    imap.login(&cfg.user, pw)?;
    imap.run("SELECT INBOX")?;
    Ok(imap)
}

fn sync_mail(cfg: &Gmail, pw: &str, cache: &mut HashMap<u32, Item>) -> Result<Vec<Item>, String> {
    let mut imap = session(cfg, pw)?;
    let mut uids = search_results(&imap.run("UID SEARCH UNSEEN")?);
    uids.sort_unstable();
    let keep = uids.len().saturating_sub(cfg.max);
    let uids = uids.split_off(keep);
    let missing: Vec<u32> = uids.iter().copied().filter(|u| !cache.contains_key(u)).collect();
    for chunk in missing.chunks(20) {
        let set: Vec<String> = chunk.iter().map(u32::to_string).collect();
        let fetched = imap.run(&format!("UID FETCH {} (UID X-GM-THRID BODY.PEEK[]<0.{MAIL_BYTES}>)", set.join(",")))?;
        for r in fetched {
            let (Some(uid), Some(raw)) = (r.number("UID"), r.literals.first()) else { continue };
            if let Some(item) = parse_mail(uid as u32, r.number("X-GM-THRID"), raw, cfg.account) {
                cache.insert(uid as u32, item);
            }
        }
    }
    cache.retain(|uid, _| uids.contains(uid));
    imap.logout();
    Ok(cache.values().cloned().collect())
}

fn mark_read(cfg: &Gmail, pw: &str, uid: u32) -> Result<(), String> {
    let mut imap = session(cfg, pw)?;
    imap.run(&format!("UID STORE {uid} +FLAGS.SILENT (\\Seen)"))?;
    imap.logout();
    Ok(())
}

fn mark_unread(cfg: &Gmail, pw: &str, uid: u32) -> Result<(), String> {
    let mut imap = session(cfg, pw)?;
    imap.run(&format!("UID STORE {uid} -FLAGS.SILENT (\\Seen)"))?;
    imap.logout();
    Ok(())
}

pub fn parse_mail(uid: u32, thread: Option<u64>, raw: &[u8], account: u8) -> Option<Item> {
    let msg = MessageParser::default().parse(raw)?;
    let from = msg.from().and_then(|a| a.first());
    let source = from.and_then(|a| a.name().or(a.address())).unwrap_or("(unknown sender)").trim().trim_matches('"').to_string();
    let title = msg.subject().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("(no subject)").to_string();
    // The plain-text part when the sender wrote one, not one derived from HTML.
    let real_text = msg.text_body.first().and_then(|&i| msg.parts.get(i as usize)).is_some_and(|p| matches!(p.body, PartType::Text(_)));
    let body = match (real_text, msg.body_text(0), msg.body_html(0)) {
        (true, Some(text), _) => text_to_md(&text),
        (_, _, Some(html)) => html_to_md(&html),
        (_, Some(text), None) => text_to_md(&text),
        _ => String::new(),
    };
    let url = match (thread, msg.message_id()) {
        (Some(t), _) => format!("https://mail.google.com/mail/u/{account}/#all/{t:x}"),
        (None, Some(id)) => format!("https://mail.google.com/mail/u/{account}/#search/rfc822msgid%3A{id}"),
        _ => format!("https://mail.google.com/mail/u/{account}/#inbox"),
    };
    Some(Item {
        id: format!("mail:{uid}"),
        kind: Kind::Mail,
        origin: GMAIL.into(),
        source,
        title,
        preview: plain(&body, PREVIEW),
        body,
        url,
        thumb: None,
        time: msg.date().map(|d| d.to_timestamp()).unwrap_or_else(now),
        uid: Some(uid),
        views: None,
        scores: Default::default(),
        icon: None,
    })
}

// ── Feeds (RSS, Atom, YouTube) ───────────────────────────────────────────

pub fn feeds(cfg: Feed, tx: Sender<Msg>) -> Sender<Cmd> {
    let every = Duration::from_secs(cfg.refresh_minutes.max(1) * 60);
    worker(every, (), move |_, _| {
        // Read every time: a feed added while running shows up on the next round.
        let list = config::feeds();
        if list.is_empty() {
            return;
        }
        let _ = tx.send(Msg::Busy { mail: false, busy: true });
        let queue = Arc::new(Mutex::new(list));
        let threads: Vec<_> = (0..FEED_THREADS)
            .map(|_| {
                let (queue, tx, cfg) = (queue.clone(), tx.clone(), cfg.clone());
                std::thread::spawn(move || {
                    while let Some((url, name)) = queue.lock().ok().and_then(|mut q| q.pop()) {
                        let result = fetch_feed(&url, &name, &cfg);
                        let _ = tx.send(Msg::Fetched { origin: url, result });
                    }
                })
            })
            .collect();
        for t in threads {
            let _ = t.join();
        }
        let _ = tx.send(Msg::Busy { mail: false, busy: false });
    })
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout(Duration::from_secs(20)).redirects(5).user_agent(concat!("omafeed/", env!("CARGO_PKG_VERSION"))).build()
}

/// YouTube answers anything that does not look like a browser with a 404 and
/// an empty feed, and now and then does so to browsers too.
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

fn fetch_feed(url: &str, name: &str, cfg: &Feed) -> Result<Vec<Item>, String> {
    let youtube = url.contains("youtube.com/feeds/");
    let mut last = String::new();
    for attempt in 0..if youtube { 3 } else { 1 } {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(700 * attempt));
        }
        let request = agent().get(url);
        let request = if youtube { request.set("User-Agent", BROWSER) } else { request };
        let response = match request.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => {
                last = format!("{url}: HTTP {code}");
                continue;
            }
            Err(ureq::Error::Transport(t)) => return Err(format!("{url}: {}", t.kind())),
        };
        let mut bytes = Vec::new();
        response.into_reader().take(FEED_BYTES).read_to_end(&mut bytes).map_err(|e| format!("{url}: {e}"))?;
        let feed = feed_rs::parser::parse(bytes.as_slice()).map_err(|e| format!("{url}: {e}"))?;
        // An empty answer is a hiccup, not a channel that deleted everything:
        // keep what we have rather than wipe it.
        if youtube && feed.entries.is_empty() {
            last = format!("{url}: empty answer");
            continue;
        }
        return Ok(items_of(feed, url, name, cfg, now()));
    }
    // The feed server is down: read the channel's own page instead.
    match channel_id(url) {
        Some(id) if youtube => channel_page(url, id, name, cfg).map_err(|e| format!("{last}; channel page: {e}")),
        _ => Err(last),
    }
}

/// A video's description, from its watch page. The channel page lists
/// videos without them, so this is fetched when one is opened up.
pub fn video_description(url: &str) -> Result<String, String> {
    let response = agent()
        .get(url)
        .set("User-Agent", BROWSER)
        .set("Accept-Language", "en-US,en;q=0.9")
        .set("Cookie", "SOCS=CAI")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response.into_reader().take(FEED_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    description_on_page(&String::from_utf8_lossy(&bytes)).ok_or_else(|| "YouTube did not include the description".into())
}

fn description_on_page(html: &str) -> Option<String> {
    let at = html.find("\"shortDescription\":")? + "\"shortDescription\":".len();
    // One JSON string, escapes and all; the rest of the page is left unread.
    serde_json::Deserializer::from_str(&html[at..]).into_iter::<String>().next()?.ok()
}

fn channel_id(url: &str) -> Option<&str> {
    let at = url.find("channel_id=")? + "channel_id=".len();
    url[at..].split('&').next().filter(|id| !id.is_empty())
}

/// A channel's latest uploads, from the data its `/videos` page is built from.
fn channel_page(feed: &str, id: &str, name: &str, cfg: &Feed) -> Result<Vec<Item>, String> {
    let page = format!("https://www.youtube.com/channel/{id}/videos");
    let response = agent()
        .get(&page)
        .set("User-Agent", BROWSER)
        .set("Accept-Language", "en-US,en;q=0.9")
        // Skips the cookie-consent interstitial served in the EU.
        .set("Cookie", "SOCS=CAI")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response.into_reader().take(FEED_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let html = String::from_utf8_lossy(&bytes);
    let channel = if name.is_empty() { meta(&html, "og:title").unwrap_or_default() } else { name.to_string() };
    // Not one video, however old: a page we could not read, not an empty channel.
    // An error keeps the videos we already have.
    if videos_on_page(&html, feed, &channel, &Feed { days: 36500, ..cfg.clone() }, now()).is_empty() {
        return Err("no videos found on the channel page".into());
    }
    Ok(videos_on_page(&html, feed, &channel, cfg, now()))
}

fn videos_on_page(html: &str, feed: &str, channel: &str, cfg: &Feed, now: i64) -> Vec<Item> {
    let Some(start) = html.find("var ytInitialData = ").map(|i| i + "var ytInitialData = ".len()) else { return Vec::new() };
    let Some(len) = html[start..].find(";</script>") else { return Vec::new() };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&html[start..start + len]) else { return Vec::new() };
    // YouTube serves two layouts and switches between them: take either.
    let mut found: Vec<(&str, String, Vec<&str>)> = Vec::new();
    let mut lockups = Vec::new();
    collect(&data, "lockupViewModel", &mut lockups);
    for v in lockups.into_iter().filter(|v| v["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO") {
        let meta = &v["metadata"]["lockupMetadataViewModel"];
        let (Some(id), Some(title)) = (v["contentId"].as_str(), meta["title"]["content"].as_str()) else { continue };
        let rows = meta["metadata"]["contentMetadataViewModel"]["metadataRows"].as_array().into_iter().flatten();
        // The text is sometimes compact ("14h ago", "273K"); the accessibility
        // label next to it keeps the words ("14 hours ago", "273 thousand views").
        let parts = rows
            .flat_map(|row| row["metadataParts"].as_array().into_iter().flatten())
            .flat_map(|part| [part["accessibilityLabel"].as_str(), part["text"]["content"].as_str()])
            .flatten()
            .collect();
        found.push((id, title.to_string(), parts));
    }
    let mut renderers = Vec::new();
    collect(&data, "videoRenderer", &mut renderers);
    for v in renderers {
        let Some(id) = v["videoId"].as_str() else { continue };
        let title = v["title"]["runs"].as_array().into_iter().flatten().filter_map(|r| r["text"].as_str()).collect::<String>();
        let parts = [&v["publishedTimeText"]["simpleText"], &v["viewCountText"]["simpleText"]].into_iter().filter_map(|t| t.as_str()).collect();
        found.push((id, title, parts));
    }
    let oldest = now - cfg.days * 86400;
    found
        .into_iter()
        .filter_map(|(id, title, parts)| {
            // Upcoming premieres have no "… ago" yet: not out, not in the feed.
            let time = now - parts.iter().find_map(|p| ago(p))?;
            (time >= oldest).then(|| Item {
                id: format!("video:yt:video:{id}"),
                kind: Kind::Video,
                origin: feed.to_string(),
                source: channel.to_string(),
                title,
                preview: String::new(),
                body: String::new(),
                url: format!("https://www.youtube.com/watch?v={id}"),
                thumb: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
                time,
                uid: None,
                views: parts.iter().find_map(|p| views(p)),
                scores: Default::default(),
                icon: None,
            })
        })
        .collect()
}

fn collect<'a>(value: &'a serde_json::Value, key: &str, out: &mut Vec<&'a serde_json::Value>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    out.push(v);
                }
                collect(v, key, out);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, key, out)),
        _ => {}
    }
}

/// "14 hours ago", "Streamed 2 days ago", "14h ago", "3mo ago" → seconds.
fn ago(text: &str) -> Option<i64> {
    let words: Vec<&str> = text.strip_suffix(" ago")?.split_whitespace().collect();
    let (n, unit) = match words[..] {
        [.., n, unit] if n.parse::<i64>().is_ok() => (n, unit),
        // Compact: the unit stuck to the number.
        [.., both] => both.split_at(both.find(|c: char| !c.is_ascii_digit())?),
        _ => return None,
    };
    let n: i64 = n.parse().ok()?;
    let unit = match unit.trim_end_matches('s') {
        "second" | "sec" => 1,
        "minute" | "min" | "m" => 60,
        "hour" | "hr" | "h" => 3600,
        "day" | "d" => 86400,
        "week" | "wk" | "w" => 7 * 86400,
        "month" | "mo" => 30 * 86400,
        "year" | "yr" | "y" => 365 * 86400,
        _ => return None,
    };
    Some(n * unit)
}

/// "273K views", "1.2M views", "1,234 views", "No views", "273 thousand views".
fn views(text: &str) -> Option<u64> {
    let n = text.strip_suffix(" views").or(text.strip_suffix(" view"))?.replace(',', "");
    if n == "No" {
        return Some(0);
    }
    let (digits, times) = match n.split_once(' ') {
        Some((d, "thousand")) => (d, 1e3),
        Some((d, "million")) => (d, 1e6),
        Some((d, "billion")) => (d, 1e9),
        Some(_) => return None,
        None => match n.chars().last()? {
            'K' => (&n[..n.len() - 1], 1e3),
            'M' => (&n[..n.len() - 1], 1e6),
            'B' => (&n[..n.len() - 1], 1e9),
            _ => (n.as_str(), 1.0),
        },
    };
    Some((digits.parse::<f64>().ok()? * times) as u64)
}

/// What `--add` follows: the feed itself, or the feed a page points to (a
/// blog's front page, a YouTube channel page). Returns (feed url, title, entries).
pub fn discover(url: &str) -> Result<(String, String, usize), String> {
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        // YouTube's feeds 404 at random: give them a few goes.
        let tries = if url.contains("youtube.com/feeds/") { 4 } else { 1 };
        let mut last = String::new();
        for attempt in 0..tries {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(700 * attempt));
            }
            match agent().get(url).set("User-Agent", BROWSER).call() {
                Ok(response) => {
                    let mut bytes = Vec::new();
                    response.into_reader().take(FEED_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    return Ok(bytes);
                }
                Err(ureq::Error::Status(code, _)) => last = format!("the server answered HTTP {code}"),
                Err(ureq::Error::Transport(t)) => return Err(format!("could not reach it ({})", t.kind())),
            }
        }
        Err(last)
    };
    let describe = |url: &str, feed: feed_rs::model::Feed| (url.to_string(), feed.title.map(|t| t.content).unwrap_or_default(), feed.entries.len());
    let page = fetch(url)?;
    if let Ok(feed) = feed_rs::parser::parse(page.as_slice()) {
        return Ok(describe(url, feed));
    }
    let html = String::from_utf8_lossy(&page);
    let found = feed_link(&html, url).ok_or("that is not a feed, and the page does not point to one")?;
    match fetch(&found) {
        Ok(bytes) => {
            let feed = feed_rs::parser::parse(bytes.as_slice()).map_err(|e| format!("{found} is not a feed we can read: {e}"))?;
            Ok(describe(&found, feed))
        }
        // YouTube's feed server goes down for hours at a time. The channel
        // page vouches for the feed, so follow it; it fills in once it is back.
        Err(_) if found.contains("youtube.com/feeds/") => Ok((found, meta(&html, "og:title").unwrap_or_default(), 0)),
        Err(e) => Err(e),
    }
}

/// `<meta property="og:title" content="…">`
fn meta(html: &str, property: &str) -> Option<String> {
    let tag = html.split("<meta").skip(1).map(|t| t.split('>').next().unwrap_or("")).find(|t| t.contains(&format!("\"{property}\"")))?;
    let at = tag.find("content=\"")? + 9;
    Some(tag[at..].split('"').next()?.replace("&amp;", "&"))
}

/// `<link rel="alternate" type="application/rss+xml" href="…">` in a page.
fn feed_link(html: &str, base: &str) -> Option<String> {
    let href = html.split("<link").skip(1).map(|tag| tag.split('>').next().unwrap_or("")).find_map(|tag| {
        if !(tag.contains("application/rss+xml") || tag.contains("application/atom+xml")) {
            return None;
        }
        let at = tag.find("href=")? + 5;
        let quote = tag[at..].chars().next()?;
        let rest = &tag[at + 1..];
        Some(rest[..rest.find(quote)?].replace("&amp;", "&"))
    })?;
    if href.starts_with("http") {
        return Some(href);
    }
    let origin_end = base.find("://").map(|i| i + 3).and_then(|i| base[i..].find('/').map(|j| i + j)).unwrap_or(base.len());
    Some(if href.starts_with('/') { format!("{}{href}", &base[..origin_end]) } else { format!("{}/{href}", base.trim_end_matches('/')) })
}

/// The icon a feed declares (Atom's `<icon>`), or its site's `/favicon.ico`.
/// The site is the feed's own link to it, not the feed's address: hnrss.org
/// points at news.ycombinator.com.
fn site_icon(feed: &feed_rs::model::Feed, url: &str) -> Option<String> {
    if let Some(icon) = feed.icon.as_ref().map(|i| i.uri.clone()).filter(|u| u.starts_with("http")) {
        return Some(icon);
    }
    let site = feed.links.iter().find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate") && l.href.starts_with("http")).map_or(url, |l| l.href.as_str());
    let (scheme, rest) = site.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next().filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}/favicon.ico"))
}

fn items_of(feed: feed_rs::model::Feed, url: &str, name: &str, cfg: &Feed, now: i64) -> Vec<Item> {
    let youtube = url.contains("youtube.com/feeds/");
    let icon = if youtube { None } else { site_icon(&feed, url) };
    let feed_title = if name.is_empty() { feed.title.map(|t| t.content).unwrap_or_else(|| url.to_string()) } else { name.to_string() };
    let oldest = now - cfg.days * 86400;
    let mut out = Vec::new();
    for e in feed.entries {
        let time = e.published.or(e.updated).map_or(now, |d| d.timestamp());
        let link = e.links.iter().find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate")).or(e.links.first()).map(|l| l.href.clone()).unwrap_or_default();
        if time < oldest || link.is_empty() || (youtube && cfg.hide_shorts && link.contains("/shorts/")) {
            continue;
        }
        let media = e.media.first();
        let title = e.title.map(|t| t.content).or_else(|| media.and_then(|m| m.title.clone()).map(|t| t.content)).unwrap_or_default();
        let body = if youtube {
            media.and_then(|m| m.description.as_ref()).map(|d| text_to_md(&d.content)).unwrap_or_default()
        } else {
            let html = e.content.and_then(|c| c.body).or(e.summary.map(|s| s.content)).unwrap_or_default();
            html_to_md(&html)
        };
        let kind = if youtube { Kind::Video } else { Kind::Post };
        let source = match (youtube, e.authors.first()) {
            (true, Some(a)) => a.name.clone(),
            _ => feed_title.clone(),
        };
        out.push(Item {
            id: format!("{}:{}", if youtube { "video" } else { "post" }, e.id),
            kind,
            origin: url.to_string(),
            source,
            title: html_to_md(&title).trim().to_string(),
            preview: plain(&body, PREVIEW),
            body,
            url: link,
            thumb: media.and_then(|m| m.thumbnails.first()).map(|t| t.image.uri.clone()),
            time,
            uid: None,
            views: media.and_then(|m| m.community.as_ref()).and_then(|c| c.stats_views),
            scores: Default::default(),
            icon: icon.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const YOUTUBE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
 <title>Some Channel</title>
 <entry>
  <id>yt:video:abc123</id>
  <yt:videoId>abc123</yt:videoId>
  <title>A long video</title>
  <link rel="alternate" href="https://www.youtube.com/watch?v=abc123"/>
  <author><name>Some Channel</name></author>
  <published>2026-09-22T10:00:00+00:00</published>
  <media:group>
   <media:title>A long video</media:title>
   <media:thumbnail url="https://i2.ytimg.com/vi/abc123/hqdefault.jpg" width="480" height="360"/>
   <media:description>What it is about.</media:description>
   <media:community><media:statistics views="12345"/></media:community>
  </media:group>
 </entry>
 <entry>
  <id>yt:video:short1</id>
  <title>A short</title>
  <link rel="alternate" href="https://www.youtube.com/shorts/short1"/>
  <published>2026-09-22T11:00:00+00:00</published>
 </entry>
 <entry>
  <id>yt:video:old</id>
  <title>Ancient</title>
  <link rel="alternate" href="https://www.youtube.com/watch?v=old"/>
  <published>2020-01-01T00:00:00+00:00</published>
 </entry>
</feed>"#;

    #[test]
    fn youtube_entries_become_videos() {
        let feed = feed_rs::parser::parse(YOUTUBE.as_bytes()).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-23T10:00:00Z").unwrap().timestamp();
        let url = "https://www.youtube.com/feeds/videos.xml?channel_id=UCx";
        let items = items_of(feed, url, "", &Feed::default(), now);
        assert_eq!(items.len(), 1, "the short and the old one are left out");
        let v = &items[0];
        assert_eq!((v.kind, v.source.as_str(), v.title.as_str()), (Kind::Video, "Some Channel", "A long video"));
        assert_eq!(v.url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(v.thumb.as_deref(), Some("https://i2.ytimg.com/vi/abc123/hqdefault.jpg"));
        assert_eq!(v.views, Some(12345));
        assert_eq!(v.preview, "What it is about.");
    }

    #[test]
    fn rss_posts_keep_their_html_as_markdown() {
        let rss = r#"<rss version="2.0"><channel><title>Blog</title>
          <item><title>Hello &amp; welcome</title><link>https://blog.x/1</link><guid>1</guid>
          <pubDate>Tue, 22 Sep 2026 10:00:00 GMT</pubDate>
          <description>&lt;p&gt;Some &lt;b&gt;news&lt;/b&gt;&lt;/p&gt;</description></item></channel></rss>"#;
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-23T10:00:00Z").unwrap().timestamp();
        let items = items_of(feed_rs::parser::parse(rss.as_bytes()).unwrap(), "https://blog.x/rss", "", &Feed::default(), now);
        assert_eq!(items[0].title, "Hello & welcome");
        assert_eq!(items[0].source, "Blog");
        assert_eq!(items[0].body.trim(), "Some **news**");
        assert_eq!(items[0].preview, "Some news");
        assert_eq!(items[0].icon.as_deref(), Some("https://blog.x/favicon.ico"), "no <link> in the feed: the feed's own host");

        // The site the feed links to, not where the feed is served from.
        let hn = r#"<rss version="2.0"><channel><title>HN</title><link>https://news.ycombinator.com/</link>
          <item><title>x</title><link>https://a.b/1</link><pubDate>Tue, 22 Sep 2026 10:00:00 GMT</pubDate></item></channel></rss>"#;
        let items = items_of(feed_rs::parser::parse(hn.as_bytes()).unwrap(), "https://hnrss.org/best", "", &Feed::default(), now);
        assert_eq!(items[0].icon.as_deref(), Some("https://news.ycombinator.com/favicon.ico"));
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><icon>https://a.b/icon.png</icon>
          <entry><id>1</id><title>t</title><link href="https://a.b/1"/><updated>2026-09-22T10:00:00Z</updated></entry></feed>"#;
        let items = items_of(feed_rs::parser::parse(atom.as_bytes()).unwrap(), "https://a.b/feed", "", &Feed::default(), now);
        assert_eq!(items[0].icon.as_deref(), Some("https://a.b/icon.png"), "a declared icon wins");
    }

    #[test]
    fn reads_a_channel_page_when_the_feed_is_down() {
        assert_eq!(ago("14 hours ago"), Some(14 * 3600));
        assert_eq!(ago("Streamed 2 days ago"), Some(2 * 86400));
        assert_eq!(ago("1 week ago"), Some(7 * 86400));
        assert_eq!(ago("Scheduled for 9/30/26"), None);
        assert_eq!(views("273K views"), Some(273_000));
        assert_eq!(views("1.2M views"), Some(1_200_000));
        assert_eq!(views("1,234 views"), Some(1234));
        assert_eq!(views("No views"), Some(0));
        assert_eq!(views("273 thousand views"), Some(273_000));
        assert_eq!(views("1.5 million views"), Some(1_500_000));
        assert_eq!(views("273K"), None, "no unit, no guess");
        assert_eq!(ago("14h ago"), Some(14 * 3600));
        assert_eq!(ago("2w ago"), Some(14 * 86400));
        assert_eq!(ago("3mo ago"), Some(90 * 86400));
        assert_eq!(ago("5m ago"), Some(300));
        assert_eq!(channel_id("https://www.youtube.com/feeds/videos.xml?channel_id=UCx"), Some("UCx"));
        let watch = r#"..."videoId":"x","shortDescription":"Line one\nhttps://a.b \"quoted\"","isCrawlable":true..."#;
        assert_eq!(description_on_page(watch).as_deref(), Some("Line one\nhttps://a.b \"quoted\""));
        assert_eq!(description_on_page("<html></html>"), None);

        let lockup = |id: &str, title: &str, when: &str| {
            format!(r#"{{"lockupViewModel":{{"contentId":"{id}","contentType":"LOCKUP_CONTENT_TYPE_VIDEO","metadata":{{"lockupMetadataViewModel":{{"title":{{"content":"{title}"}},"metadata":{{"contentMetadataViewModel":{{"metadataRows":[{{"metadataParts":[{{"text":{{"content":"273K views"}}}},{{"text":{{"content":"{when}"}}}}]}}]}}}}}}}}}}}}"#)
        };
        let html = format!(
            "<script>var ytInitialData = {{\"items\":[{},{},{}]}};</script>",
            lockup("abc", "New one", "14 hours ago"),
            lockup("old", "Old one", "2 months ago"),
            lockup("soon", "Premiere", "Scheduled for tomorrow")
        );
        let items = videos_on_page(&html, "https://www.youtube.com/feeds/videos.xml?channel_id=UCx", "Chan", &Feed::default(), 1_000_000_000);
        assert_eq!(items.len(), 1, "only what is out and recent");
        let v = &items[0];
        assert_eq!((v.id.as_str(), v.title.as_str(), v.source.as_str()), ("video:yt:video:abc", "New one", "Chan"));
        assert_eq!(v.time, 1_000_000_000 - 14 * 3600);
        assert_eq!(v.views, Some(273_000));
        assert_eq!(v.thumb.as_deref(), Some("https://i.ytimg.com/vi/abc/hqdefault.jpg"));

        let compact = r#"<script>var ytInitialData = {"x":[{"lockupViewModel":{"contentId":"c1","contentType":"LOCKUP_CONTENT_TYPE_VIDEO","metadata":{"lockupMetadataViewModel":{"title":{"content":"Compact"},"metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"273K"},"accessibilityLabel":"273 thousand views"},{"text":{"content":"14h ago"},"accessibilityLabel":"14 hours ago"}]}]}}}}}}]};</script>"#;
        let items = videos_on_page(compact, "f", "Chan", &Feed::default(), 1_000_000_000);
        assert_eq!((items[0].time, items[0].views), (1_000_000_000 - 14 * 3600, Some(273_000)));

        let older_layout = r#"<script>var ytInitialData = {"x":[{"videoRenderer":{"videoId":"v1","title":{"runs":[{"text":"Old "},{"text":"layout"}]},"publishedTimeText":{"simpleText":"3 days ago"},"viewCountText":{"simpleText":"1,234 views"}}}]};</script>"#;
        let items = videos_on_page(older_layout, "f", "Chan", &Feed::default(), 1_000_000_000);
        assert_eq!((items[0].title.as_str(), items[0].views), ("Old layout", Some(1234)));
    }

    /// `cargo test -- --ignored live_youtube --nocapture`: the real thing, over the network.
    #[test]
    #[ignore]
    fn live_youtube() {
        let feed = "https://www.youtube.com/feeds/videos.xml?channel_id=UC9x0AN7BWHpCDHSm9NiJFJQ";
        for _ in 0..3 {
            let page = channel_page(feed, "UC9x0AN7BWHpCDHSm9NiJFJQ", "", &Feed { days: 60, ..Feed::default() });
            println!("channel page: {:?} videos", page.as_ref().map(|v| v.len()));
        }
        let items = fetch_feed(feed, "", &Feed::default()).unwrap();
        for i in &items {
            println!("{} · {} · {:?} views · {}", i.source, i.title, i.views, crate::model::age(i.time, now()));
        }
        assert!(!items.is_empty());
        let about = video_description(&items[0].url).unwrap();
        println!("description: {}", about.lines().take(3).collect::<Vec<_>>().join(" / "));
        assert!(!about.is_empty());
    }

    #[test]
    fn finds_the_feed_a_page_points_to() {
        let yt = r#"<head><link rel="alternate" type="application/rss+xml" title="RSS" href="https://www.youtube.com/feeds/videos.xml?channel_id=UCx&amp;a=1"></head>"#;
        assert_eq!(feed_link(yt, "https://www.youtube.com/@x").as_deref(), Some("https://www.youtube.com/feeds/videos.xml?channel_id=UCx&a=1"));
        let blog = "<link rel='stylesheet' href='/s.css'><link rel='alternate' type='application/atom+xml' href='/feed.xml'>";
        assert_eq!(feed_link(blog, "https://blog.x/posts/1").as_deref(), Some("https://blog.x/feed.xml"));
        assert_eq!(feed_link("<p>nothing</p>", "https://a.b"), None);
        assert_eq!(meta(r#"<meta property="og:title" content="Rock &amp; Roll">"#, "og:title").as_deref(), Some("Rock & Roll"));
    }

    #[test]
    fn mail_is_read_from_its_plain_part() {
        let raw = b"From: \"Jane Doe\" <jane@x.y>\r\nSubject: Lunch?\r\nDate: Tue, 22 Sep 2026 10:00:00 +0000\r\nMessage-ID: <m1@x.y>\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nAre you free *tomorrow*?\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>HTML version</p>\r\n--b--\r\n";
        let item = parse_mail(7, Some(0x19a2b3c4d5e6f708), raw, 0).unwrap();
        assert_eq!((item.source.as_str(), item.title.as_str()), ("Jane Doe", "Lunch?"));
        assert_eq!(item.url, "https://mail.google.com/mail/u/0/#all/19a2b3c4d5e6f708");
        assert!(item.body.contains("Are you free *tomorrow*?"));
        assert_eq!(item.preview, "Are you free tomorrow?");
        assert_eq!(item.uid, Some(7));

        let html_only = b"From: news@x.y\r\nSubject: News\r\nContent-Type: text/html\r\n\r\n<table><tr><td><h2>Top</h2><p>story</p></td></tr></table>";
        let item = parse_mail(8, None, html_only, 1).unwrap();
        assert_eq!(item.icon, None);
        assert_eq!(item.source, "news@x.y");
        assert!(item.body.contains("## Top"), "{}", item.body);
        assert!(item.url.starts_with("https://mail.google.com/mail/u/1/"));
    }
}
