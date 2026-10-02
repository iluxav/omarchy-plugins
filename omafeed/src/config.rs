//! Settings live in `~/.config/omafeed/`:
//!
//! - `config.toml`: the Gmail account and a few knobs.
//! - `feeds`: one feed URL per line, `# anything` after it is a comment.
//!   YouTube channels are feeds too (`--import-youtube` writes them here).

use std::path::PathBuf;

use serde::Deserialize;

const TEMPLATE: &str = r#"# omafeed settings

# Unread Gmail, over IMAP. Needs an app password:
# https://myaccount.google.com/apppasswords
# [gmail]
# user = "you@gmail.com"
# password_command = "cat ~/.config/omafeed/gmail-password"   # or: password = "abcd efgh ijkl mnop"
# account = 0          # the N in mail.google.com/mail/u/N, for links
# max = 50             # newest unread mails to show
# poll_seconds = 60

# Prisms by jev (https://typesafe.ai); the prisms themselves are in prisms.yaml.
# The key can also come from TYPESAFE_API_KEY in the environment.
# [jev]
# api_key_command = "cat ~/.config/omafeed/jev-key"   # or: api_key = "..."

[feed]
days = 7               # how far back feeds and videos go
refresh_minutes = 15
hide_shorts = true
notify = true          # desktop notification for anything new
"#;

const FEEDS_TEMPLATE: &str = "# One feed per line. RSS, Atom, or a YouTube channel:
#   https://www.youtube.com/feeds/videos.xml?channel_id=UC...
# omafeed --import-youtube subscriptions.csv adds all your subscriptions.
# omafeed --add <url> adds one.
";

#[derive(Deserialize, Default)]
pub struct Config {
    pub gmail: Option<Gmail>,
    #[serde(default)]
    pub feed: Feed,
    #[serde(default)]
    pub jev: Jev,
}

#[derive(Deserialize, Clone)]
pub struct Jev {
    pub api_key: Option<String>,
    pub api_key_command: Option<String>,
    #[serde(default = "jev_latest")]
    pub model: String,
}

impl Default for Jev {
    fn default() -> Self {
        Jev { api_key: None, api_key_command: None, model: jev_latest() }
    }
}

fn jev_latest() -> String {
    "jev-latest".into()
}

impl Jev {
    /// `TYPESAFE_API_KEY`, then `api_key`, then `api_key_command`. None: no prisms.
    pub fn key(&self) -> Option<String> {
        let from_env = std::env::var("TYPESAFE_API_KEY").ok();
        let from_cmd = || {
            let out = std::process::Command::new("sh").args(["-c", self.api_key_command.as_deref()?]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        from_env.or_else(|| self.api_key.clone()).or_else(from_cmd).filter(|k| !k.trim().is_empty())
    }
}

#[derive(Deserialize, Clone)]
pub struct Gmail {
    pub user: String,
    pub password: Option<String>,
    pub password_command: Option<String>,
    #[serde(default)]
    pub account: u8,
    #[serde(default = "fifty")]
    pub max: usize,
    #[serde(default = "sixty")]
    pub poll_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub struct Feed {
    #[serde(default = "seven")]
    pub days: i64,
    #[serde(default = "fifteen")]
    pub refresh_minutes: u64,
    #[serde(default = "yes")]
    pub hide_shorts: bool,
    #[serde(default = "yes")]
    pub notify: bool,
}

impl Default for Feed {
    fn default() -> Self {
        Feed { days: 7, refresh_minutes: 15, hide_shorts: true, notify: true }
    }
}

fn fifty() -> usize {
    50
}
fn sixty() -> u64 {
    60
}
fn seven() -> i64 {
    7
}
fn fifteen() -> u64 {
    15
}
fn yes() -> bool {
    true
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback)).join("omafeed")
}

pub fn dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

pub fn feeds_file() -> PathBuf {
    dir().join("feeds")
}

/// The settings, writing the commented templates on first run.
pub fn load() -> Result<Config, String> {
    let dir = dir();
    let file = dir.join("config.toml");
    if !file.exists() {
        let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, TEMPLATE));
    }
    if !feeds_file().exists() {
        let _ = std::fs::write(feeds_file(), FEEDS_TEMPLATE);
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    toml::from_str(&text).map_err(|e| format!("{}: {}", file.display(), e.message()))
}

/// `(url, name)` for each line of the feeds file.
pub fn feeds() -> Vec<(String, String)> {
    parse_feeds(&std::fs::read_to_string(feeds_file()).unwrap_or_default())
}

fn parse_feeds(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (url, name) = line.split_once('#').unwrap_or((line, ""));
            let url = url.trim();
            url.starts_with("http").then(|| (url.to_string(), name.trim().to_string()))
        })
        .collect()
}

/// Append feeds not already there. Returns how many were new.
pub fn add_feeds(new: &[(String, String)]) -> std::io::Result<usize> {
    let file = feeds_file();
    std::fs::create_dir_all(dir())?;
    let mut text = std::fs::read_to_string(&file).unwrap_or_else(|_| FEEDS_TEMPLATE.to_string());
    let have: Vec<String> = parse_feeds(&text).into_iter().map(|(u, _)| u).collect();
    let mut added = 0;
    for (url, name) in new {
        if have.contains(url) {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&if name.is_empty() { url.clone() } else { format!("{url}  # {name}") });
        text.push('\n');
        added += 1;
    }
    std::fs::write(&file, text)?;
    Ok(added)
}

/// The channels in a Google Takeout `subscriptions.csv`
/// (`Channel Id,Channel Url,Channel Title`), as feeds.
pub fn youtube_takeout(csv: &str) -> Vec<(String, String)> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.splitn(3, ',');
            let id = cols.next()?.trim();
            let _url = cols.next()?;
            let title = cols.next().unwrap_or("").trim().trim_matches('"');
            id.starts_with("UC").then(|| (format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}"), title.to_string()))
        })
        .collect()
}

impl Gmail {
    pub fn password(&self) -> Result<String, String> {
        if let Some(pw) = &self.password {
            return Ok(pw.clone());
        }
        let cmd = self.password_command.as_deref().ok_or("gmail needs `password` or `password_command` in config.toml")?;
        let out = std::process::Command::new("sh").args(["-c", cmd]).output().map_err(|e| format!("password_command: {e}"))?;
        let pw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || pw.is_empty() {
            return Err(format!("password_command gave no password: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(pw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_template_and_feed_lines() {
        let config: Config = toml::from_str(TEMPLATE).unwrap();
        assert!(config.gmail.is_none());
        assert_eq!(config.feed.days, 7);
        let gmail: Config = toml::from_str("[gmail]\nuser = \"a@b.c\"\npassword = \"x\"").unwrap();
        let g = gmail.gmail.unwrap();
        assert_eq!((g.max, g.poll_seconds, g.account), (50, 60, 0));
        assert_eq!(g.password().unwrap(), "x");

        let feeds = parse_feeds("# comment\nhttps://a.b/rss  # A B\n\n  https://c.d/atom\nnot a url\n");
        assert_eq!(feeds, [("https://a.b/rss".into(), "A B".into()), ("https://c.d/atom".into(), String::new())]);
    }

    #[test]
    fn imports_takeout_subscriptions() {
        let csv = "Channel Id,Channel Url,Channel Title\nUCabc,http://www.youtube.com/channel/UCabc,Some, Channel\nUCdef,http://x,Other\n";
        let feeds = youtube_takeout(csv);
        assert_eq!(feeds[0], ("https://www.youtube.com/feeds/videos.xml?channel_id=UCabc".into(), "Some, Channel".into()));
        assert_eq!(feeds.len(), 2);
    }
}
