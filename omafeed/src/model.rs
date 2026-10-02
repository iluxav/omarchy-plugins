//! What the feed is made of. Every source turns its things into `Item`s, and
//! the rest of the app never needs to know where one came from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Mail,
    Video,
    Post,
}

impl Kind {
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Mail => "󰇮",
            Kind::Video => "󰗃",
            Kind::Post => "󰑫",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    /// Unique across sources: `mail:…`, `video:…`, `post:…`.
    pub id: String,
    pub kind: Kind,
    /// Which fetch this came from (`gmail`, or a feed's URL). A fetch replaces
    /// everything of its origin; a failed fetch leaves it alone.
    pub origin: String,
    /// Sender, channel or feed name.
    pub source: String,
    pub title: String,
    /// Plain text, one paragraph.
    pub preview: String,
    /// Markdown; empty when there is nothing to expand.
    pub body: String,
    pub url: String,
    pub thumb: Option<String>,
    /// Unix seconds.
    pub time: i64,
    /// IMAP UID, for marking a mail read.
    pub uid: Option<u32>,
    #[serde(default)]
    pub views: Option<u64>,
    /// jev's answer for each prism in `prisms.yaml`: how likely a yes. Empty: not looked at yet.
    #[serde(default)]
    pub scores: BTreeMap<String, f32>,
    /// The site's own icon, shown instead of the generic one.
    #[serde(default)]
    pub icon: Option<String>,
}

/// "now", "5m", "3h", "2d", or the date.
pub fn age(time: i64, now: i64) -> String {
    let s = (now - time).max(0);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        86400..604800 => format!("{}d", s / 86400),
        _ => chrono::DateTime::from_timestamp(time, 0).map(|d| d.format("%b %-d").to_string()).unwrap_or_default(),
    }
}

/// "1.2K", "34K", "5.6M".
pub fn count(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..10_000 => format!("{:.1}K", n as f64 / 1e3),
        10_000..1_000_000 => format!("{}K", n / 1000),
        1_000_000..10_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{}M", n / 1_000_000),
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_like_a_timeline() {
        assert_eq!(age(1000, 1030), "now");
        assert_eq!(age(1000, 1000 + 300), "5m");
        assert_eq!(age(1000, 1000 + 3 * 3600), "3h");
        assert_eq!(age(1000, 1000 + 2 * 86400), "2d");
        assert_eq!(age(1_758_000_000, 1_758_000_000 + 30 * 86400), "Sep 16");
        assert_eq!(age(2000, 1000), "now", "a clock slightly ahead is not the future");
        assert_eq!(count(999), "999");
        assert_eq!(count(1234), "1.2K");
        assert_eq!(count(56_789), "56K");
        assert_eq!(count(5_600_000), "5.6M");
    }
}
