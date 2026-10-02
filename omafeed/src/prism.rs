//! Prisms: what you are really looking for in the flood. Each one is a yes/no
//! question jev (TypeSafe's System One model) answers for every item with a
//! probability: "does this point to a candidate?" All of them go in one
//! request per item. An item can pass several prisms, or none (most won't).
//!
//! https://docs.typesafe.ai/primitives/noul

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::model::{Item, Kind};
use crate::sources::Msg;

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const WORKERS: usize = 3;
/// Enough of a body to judge it by.
const STATE_CHARS: usize = 2500;

const TEMPLATE: &str = r#"# Prisms for omafeed: what you are really looking for.
# jev reads every item and answers each prism's question with a probability.
# Items that pass show up under that prism (ctrl+p), best candidates first.
#
#   question  the yes/no question, asked of every item
#   yes       what counts as a yes: be concrete, it is what jev goes by
#   no        what looks close but is not
#
# Change this file and everything is looked at again on the next start.

threshold: 0.45   # how likely a yes must be to pass (0 to 1). Lower: more candidates, more noise.
alert: 0.8        # a new item this likely gets its own notification

prisms:
  linux tool idea:
    question: Does this point to an open-source tool someone could build for Linux users?
    yes: A pain point, missing tool, clumsy workflow, feature request or rising trend in the Linux world (desktop, terminal, Arch, Hyprland, Omarchy, dotfiles, sysadmin, self-hosting) that a new open-source project could solve
    no: Linux news, releases, benchmarks or opinions with nothing left to build; a project that already solves it well

  productivity tool idea:
    question: Does this point to an open-source productivity tool worth building?
    yes: Friction in how people take notes, plan, write, read, search, manage tasks, email or knowledge, or automate their day, that a new open-source app, CLI or plugin could remove
    no: Productivity advice or listicles with no tool in them; a mature product that already covers it

  saas idea:
    question: Does this point to a small SaaS business someone could build and charge for?
    yes: A problem people or companies would pay to have solved, an underserved niche, a manual process ripe for software, a shift that opens a market, or someone describing a business that makes money
    no: News about big companies, funding rounds or markets with no opening for a small product; free hobby tools with no one to pay

  urgent politics:
    question: Is this an urgent political development I should pay attention to right now?
    yes: A major event with real consequences soon — elections, wars and ceasefires, emergency laws, sanctions, government crises, security threats — especially in the US or Israel
    no: Routine political news, commentary, polls, campaign color, or stories with no near-term consequence

  ai to watch:
    question: Is this a new AI technology or shift I need to pay attention to?
    yes: A new model, capability, technique, tool or platform change in AI that changes what can be built or how developers work; a genuinely new release, not a rehash
    no: AI opinion pieces, hype, funding news, or incremental updates with nothing new to learn or use
"#;

#[derive(Debug)]
pub struct Prism {
    pub name: String,
    question: String,
    yes: Option<Value>,
    no: Option<Value>,
}

#[derive(Debug)]
pub struct Prisms {
    pub list: Vec<Prism>,
    /// An item passes a prism at or above this.
    pub threshold: f32,
    /// A new item at or above this gets a notification.
    pub alert: f32,
    /// Changes whenever the file does: scores from another version are stale.
    pub signature: String,
}

fn file() -> std::path::PathBuf {
    crate::config::dir().join("prisms.yaml")
}

/// The prisms, writing a starting set on first run.
pub fn load() -> Result<Prisms, String> {
    let path = file();
    if !path.exists() {
        let _ = std::fs::write(&path, TEMPLATE);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse(text: &str) -> Result<Prisms, String> {
    let doc: serde_norway::Value = serde_norway::from_str(text).map_err(|e| e.to_string())?;
    let number = |key: &str, default: f32| doc.get(key).and_then(|v| v.as_f64()).map_or(default, |v| v.clamp(0.0, 1.0) as f32);
    let map = doc.get("prisms").and_then(|v| v.as_mapping()).ok_or("expected `prisms:` with one prism per entry")?;
    let json = |v: Option<&serde_norway::Value>| v.map(serde_json::to_value).transpose().map_err(|e| e.to_string());
    let mut list = Vec::new();
    for (name, spec) in map {
        let Some(name) = name.as_str().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) else { continue };
        // `name: the question` on its own line works too.
        let (question, yes, no) = match spec.as_str() {
            Some(q) => (q.to_string(), None, None),
            None => {
                let question = spec.get("question").and_then(|q| q.as_str()).ok_or(format!("prism `{name}` needs a `question:`"))?;
                (question.to_string(), json(spec.get("yes"))?, json(spec.get("no"))?)
            }
        };
        list.push(Prism { name, question, yes, no });
    }
    if list.is_empty() {
        return Err("no prisms in it".into());
    }
    let signature = format!("{:016x}", text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)));
    Ok(Prisms { list, threshold: number("threshold", 0.45), alert: number("alert", 0.8), signature })
}

/// What jev reads: what kind of thing it is, who from, and the words.
fn state(item: &Item) -> String {
    let what = match item.kind {
        Kind::Mail => "An email from",
        Kind::Video => "A YouTube video by",
        Kind::Post => "An article from",
    };
    let body = if item.body.trim().is_empty() { &item.preview } else { &item.body };
    let body: String = body.chars().take(STATE_CHARS).collect();
    format!("{what} {}\nTitle: {}\n\n{body}", item.source, item.title)
}

fn request(prisms: &Prisms, model: &str, item: &Item) -> Value {
    let mut questions = Map::new();
    for (k, prism) in prisms.list.iter().enumerate() {
        let mut q = json!({ "type": "noul", "instructions": prism.question });
        // Only the sides that were described.
        let criteria: Map<String, Value> = [("true", &prism.yes), ("false", &prism.no)].into_iter().filter_map(|(k, v)| Some((k.to_string(), v.clone()?))).collect();
        if !criteria.is_empty() {
            q["criteria"] = Value::Object(criteria);
        }
        // Keys are ours to choose; names with spaces and symbols are not good keys.
        questions.insert(format!("p{k}"), q);
    }
    json!({ "state": state(item), "model": model, "questions": questions })
}

/// Each prism's probability of a yes for this item.
fn ask_at(endpoint: &str, key: &str, model: &str, prisms: &Prisms, item: &Item) -> Result<BTreeMap<String, f32>, String> {
    let body = request(prisms, model, item);
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout(Duration::from_secs(30)).build();
    let mut wait = Duration::from_millis(500);
    for _ in 0..4 {
        match agent.post(endpoint).set("Authorization", &format!("Bearer {key}")).send_json(&body) {
            Ok(response) => {
                let mut text = String::new();
                response.into_reader().take(1 << 20).read_to_string(&mut text).map_err(|e| e.to_string())?;
                let answer: Value = serde_json::from_str(&text).map_err(|e| format!("jev answered something unreadable: {e}"))?;
                let scores: BTreeMap<String, f32> = prisms
                    .list
                    .iter()
                    .enumerate()
                    .filter_map(|(k, p)| Some((p.name.clone(), answer["answers"][format!("p{k}")]["noul"].as_f64()? as f32)))
                    .collect();
                return if scores.is_empty() { Err("jev gave no answers".into()) } else { Ok(scores) };
            }
            // Busy: back off and try again.
            Err(ureq::Error::Status(429 | 529 | 503, _)) => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            Err(ureq::Error::Status(401, _)) => return Err("jev: the API key was refused".into()),
            Err(ureq::Error::Status(code, r)) => return Err(format!("jev: HTTP {code} {}", r.into_string().unwrap_or_default().chars().take(200).collect::<String>())),
            Err(ureq::Error::Transport(t)) => return Err(format!("jev: {}", t.kind())),
        }
    }
    Err("jev is busy; will try again later".into())
}

/// Looks at items in the background. A video without its description gets
/// it first, since a title alone says little.
pub fn spawn(prisms: Arc<Prisms>, key: String, model: String, tx: Sender<Msg>) -> Sender<Item> {
    let (jobs, queue): (Sender<Item>, Receiver<Item>) = channel();
    let queue = Arc::new(Mutex::new(queue));
    for _ in 0..WORKERS {
        let (queue, tx, prisms, key, model) = (queue.clone(), tx.clone(), prisms.clone(), key.clone(), model.clone());
        std::thread::spawn(move || {
            loop {
                let Ok(mut item) = queue.lock().map_err(drop).and_then(|q| q.recv().map_err(drop)) else { return };
                if item.kind == Kind::Video && item.body.trim().is_empty() {
                    let result = crate::sources::video_description(&item.url);
                    if let Ok(text) = &result {
                        item.body = crate::body::text_to_md(text);
                    }
                    let _ = tx.send(Msg::Description { id: item.id.clone(), result, quiet: true });
                    // Gently: these are page loads on YouTube.
                    std::thread::sleep(Duration::from_millis(300));
                }
                let result = ask_at(ENDPOINT, &key, &model, &prisms, &item);
                if tx.send(Msg::Scored { id: item.id, result }).is_err() {
                    return;
                }
            }
        });
    }
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_prisms() {
        let p = parse(TEMPLATE).unwrap();
        let names: Vec<&str> = p.list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["linux tool idea", "productivity tool idea", "saas idea", "urgent politics", "ai to watch"]);
        assert_eq!((p.threshold, p.alert), (0.45, 0.8));
        assert!(p.list[2].yes.as_ref().unwrap().as_str().unwrap().starts_with("A problem people"));

        let short = parse("prisms:\n  rust jobs: Is this a Rust job opening?\n").unwrap();
        assert_eq!((short.list[0].question.as_str(), short.threshold), ("Is this a Rust job opening?", 0.45));
        assert!(parse("prisms:\n  x:\n    yes: y\n").unwrap_err().contains("needs a `question:`"));
        assert!(parse("threshold: 0.5").is_err());
        assert_ne!(parse("prisms: {a: x}").unwrap().signature, parse("prisms: {a: y}").unwrap().signature);
    }

    /// Answer one request the way jev does, and hand back what was asked.
    fn fake_jev(status: &'static str, answer: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut head, mut len) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    len = n.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            tx.send(format!("{head}{}", String::from_utf8_lossy(&body))).unwrap();
            let mut stream = stream;
            let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (url, rx)
    }

    fn video() -> Item {
        Item {
            id: "video:1".into(),
            kind: Kind::Video,
            origin: String::new(),
            source: "NetworkChuck".into(),
            title: "I tested it".into(),
            preview: "short".into(),
            body: String::new(),
            url: String::new(),
            thumb: None,
            time: 0,
            uid: None,
            views: None,
            scores: BTreeMap::new(),
            icon: None,
        }
    }

    #[test]
    fn asks_every_prism_in_one_request() {
        let answer = r#"{"model":"jev-1.13.0","answers":{"p0":{"type":"noul","noul":0.91},"p1":{"type":"noul","noul":0.12},"p2":{"type":"noul","noul":0.66},"p3":{"type":"noul","noul":0.01},"p4":{"type":"noul","noul":0.3}},"usage":{"input_tokens":900,"output_tokens":60}}"#;
        let (url, asked) = fake_jev("200 OK", answer);
        let prisms = parse(TEMPLATE).unwrap();
        let scores = ask_at(&url, "sk-test", "jev-latest", &prisms, &video()).unwrap();
        assert_eq!(scores["linux tool idea"], 0.91);
        assert_eq!(scores["saas idea"], 0.66);
        assert_eq!(scores.len(), 5);

        let request = asked.recv().unwrap();
        assert!(request.starts_with("POST /v1/systemone ") && request.contains("Authorization: Bearer sk-test"), "{request}");
        let body: Value = serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "A YouTube video by NetworkChuck\nTitle: I tested it\n\nshort");
        let q = &body["questions"]["p2"];
        assert_eq!(q["type"], "noul");
        assert_eq!(q["instructions"], "Does this point to a small SaaS business someone could build and charge for?");
        assert!(q["criteria"]["true"].as_str().unwrap().starts_with("A problem people"));
        assert!(q["criteria"]["false"].as_str().unwrap().starts_with("News about big companies"));
        assert_eq!(body["questions"].as_object().unwrap().len(), 5);

        let one_sided = parse("prisms:\n  x:\n    question: q?\n    yes: y\n").unwrap();
        assert_eq!(super::request(&one_sided, "m", &video())["questions"]["p0"]["criteria"], json!({ "true": "y" }));
        let bare = parse("prisms:\n  x: q?\n").unwrap();
        assert!(super::request(&bare, "m", &video())["questions"]["p0"].get("criteria").is_none());
    }

    /// `cargo test -- --ignored live_prisms --nocapture`: the real jev, with the key from config.toml.
    #[test]
    #[ignore]
    fn live_prisms() {
        let key = crate::config::load().unwrap().jev.key().expect("a jev key in config.toml");
        let prisms = parse(TEMPLATE).unwrap();
        let post = |source: &str, title: &str, body: &str| Item { kind: Kind::Post, source: source.into(), title: title.into(), body: body.into(), ..video() };
        let items = [
            post("Hacker News", "Ask HN: Is there a tool to sync dotfiles across machines without git?", "I keep breaking my Hyprland config across three laptops. Stow and chezmoi feel heavy. Anyone built something simple?"),
            post("Indie Hackers", "I make $4k/month from a tiny tool that turns invoices into spreadsheets", "Accountants at small firms were retyping PDF invoices by hand. I built a converter, charged $29/month, and it grew by word of mouth."),
            post("Times of Israel", "Knesset dissolves; snap elections set for December", "The coalition collapsed overnight after the budget vote failed, triggering early elections."),
        ];
        for item in &items {
            let scores = ask_at(ENDPOINT, &key, "jev-latest", &prisms, item).unwrap();
            let shown: Vec<String> = scores.iter().map(|(n, s)| format!("{n} {:.0}%", s * 100.0)).collect();
            println!("{}\n   {}", item.title, shown.join(" · "));
        }
    }

    #[test]
    fn a_refused_key_says_so() {
        let (url, _asked) = fake_jev("401 Unauthorized", "{}");
        let err = ask_at(&url, "bad", "jev-latest", &parse(TEMPLATE).unwrap(), &video()).unwrap_err();
        assert_eq!(err, "jev: the API key was refused");
    }
}
