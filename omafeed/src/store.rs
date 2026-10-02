//! What survives a restart: the last items (so the feed is there at once),
//! which ones you have already come across, and which you cleared away.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::Item;

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub items: Vec<Item>,
    /// Ever received: anything not in here is news worth a notification.
    pub known: HashSet<String>,
    /// The cursor has been on it: no "new" dot.
    pub seen: HashSet<String>,
    /// Cleared with `enter`.
    pub hidden: HashSet<String>,
    /// Pinned with `b`: kept whole, so they outlive the feed they came from.
    #[serde(default)]
    pub pinned: Vec<Item>,
    /// Which `prisms.yaml` the stored scores came from.
    #[serde(default)]
    pub prisms: String,
    /// Cleared items as id → the item's time. A feed sends a cleared item again
    /// until it is too old for the feed, so its id has to outlive it; what it
    /// said does not. Dropped once older than `oldest` in `slim`.
    #[serde(default)]
    pub cleared: HashMap<String, i64>,
}

/// What goes to disk: items in full, except the cleared ones, which shrink
/// to their id and time. The id sets keep only ids something still refers to.
pub fn slim(state: &State, oldest: i64) -> State {
    let mut cleared: HashMap<String, i64> = state.cleared.iter().filter(|(_, time)| **time >= oldest).map(|(id, time)| (id.clone(), *time)).collect();
    let mut items = Vec::new();
    for item in &state.items {
        if state.hidden.contains(&item.id) {
            cleared.insert(item.id.clone(), item.time);
        } else {
            items.push(item.clone());
        }
    }
    let ids: HashSet<&String> = items.iter().chain(&state.pinned).map(|i| &i.id).chain(cleared.keys()).collect();
    let keep = |set: &HashSet<String>| set.iter().filter(|id| ids.contains(id)).cloned().collect();
    State {
        known: keep(&state.known),
        seen: keep(&state.seen),
        hidden: keep(&state.hidden),
        items,
        pinned: state.pinned.clone(),
        prisms: state.prisms.clone(),
        cleared,
    }
}

fn file() -> PathBuf {
    crate::config::state_dir().join("state.json")
}

/// `None` on the very first run.
pub fn load() -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(file()).ok()?).ok()
}

pub fn save(state: &State) {
    let path = file();
    let Ok(json) = serde_json::to_string(state) else { return };
    let tmp = path.with_extension("tmp");
    let _ = std::fs::create_dir_all(crate::config::state_dir()).and_then(|_| std::fs::write(&tmp, json)).and_then(|_| std::fs::rename(&tmp, &path));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    fn item(id: &str, time: i64) -> Item {
        Item { id: id.into(), kind: Kind::Post, origin: "f".into(), source: "s".into(), title: "t".into(), preview: "p".into(),
               body: "a long body".into(), url: "u".into(), thumb: None, time, uid: None, views: None, scores: Default::default(), icon: None }
    }

    #[test]
    fn cleared_items_are_kept_as_ids_only_until_too_old() {
        let mut state = State { items: vec![item("post:a", 100), item("post:b", 200)], ..Default::default() };
        for id in ["post:a", "post:b"] {
            state.known.insert(id.into());
            state.seen.insert(id.into());
        }
        state.hidden.insert("post:a".into());
        state.cleared.insert("post:old".into(), 10);
        state.hidden.insert("post:old".into());

        let saved = slim(&state, 50);
        assert_eq!(saved.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["post:b"]);
        assert_eq!(saved.cleared, HashMap::from([("post:a".to_string(), 100)]));
        assert!(saved.hidden.contains("post:a") && !saved.hidden.contains("post:old"));
        assert!(saved.known.contains("post:a") && saved.seen.contains("post:a"));

        // After a restart the cleared item is only an id, and it stays cleared.
        let again = slim(&saved, 50);
        assert_eq!(again.cleared, saved.cleared);
        assert!(again.hidden.contains("post:a"));
    }
}
