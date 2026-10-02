//! Just enough IMAP for reading unread Gmail: log in, search, fetch, flag.
//!
//! Responses are read line by line; a line ending in `{N}` is followed by N
//! bytes of literal data (a message body), which is kept apart from the text
//! around it. That is all the parsing a FETCH needs, since the fields we want
//! (`UID`, `X-GM-THRID`) sit in the plain text.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

type Tls = StreamOwned<ClientConnection, TcpStream>;

pub struct Imap {
    io: BufReader<Tls>,
    tag: u32,
}

/// One untagged response: its text, and any literals it carried.
#[derive(Debug, Default)]
pub struct Untagged {
    pub text: String,
    pub literals: Vec<Vec<u8>>,
}

impl Untagged {
    /// The number after `name ` in the text, e.g. `UID 42`.
    pub fn number(&self, name: &str) -> Option<u64> {
        let at = self.text.find(&format!("{name} "))? + name.len() + 1;
        let digits: String = self.text[at..].chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
}

fn tls_config() -> Arc<ClientConfig> {
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

pub fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Imap {
    pub fn connect(host: &str) -> Result<Imap, String> {
        let tcp = TcpStream::connect((host, 993)).map_err(|e| format!("cannot reach {host}: {e}"))?;
        tcp.set_read_timeout(Some(Duration::from_secs(60))).ok();
        tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();
        let name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
        let conn = ClientConnection::new(tls_config(), name).map_err(|e| e.to_string())?;
        let mut imap = Imap { io: BufReader::new(StreamOwned::new(conn, tcp)), tag: 0 };
        let greeting = imap.line()?;
        if !greeting.starts_with("* OK") {
            return Err(format!("unexpected greeting: {}", greeting.trim()));
        }
        Ok(imap)
    }

    fn line(&mut self) -> Result<String, String> {
        let mut raw = Vec::new();
        let n = self.io.read_until(b'\n', &mut raw).map_err(|e| format!("connection: {e}"))?;
        if n == 0 {
            return Err("the server closed the connection".into());
        }
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    /// Send a command and collect its untagged responses. `Err` on NO/BAD.
    pub fn run(&mut self, command: &str) -> Result<Vec<Untagged>, String> {
        self.tag += 1;
        let tag = format!("a{}", self.tag);
        let stream = self.io.get_mut();
        stream.write_all(format!("{tag} {command}\r\n").as_bytes()).and_then(|_| stream.flush()).map_err(|e| format!("connection: {e}"))?;

        let mut out: Vec<Untagged> = Vec::new();
        let mut current: Option<Untagged> = None;
        loop {
            let line = self.line()?;
            let continuing = current.is_some();
            if !continuing {
                if let Some(status) = line.strip_prefix(&format!("{tag} ")) {
                    return if status.starts_with("OK") { Ok(out) } else { Err(status.trim().to_string()) };
                }
            }
            let mut entry = current.take().unwrap_or_default();
            let body = line.trim_end_matches(['\r', '\n']);
            match literal_size(body) {
                Some((size, before)) => {
                    entry.text.push_str(before);
                    let mut data = vec![0; size];
                    self.io.read_exact(&mut data).map_err(|e| format!("connection: {e}"))?;
                    entry.literals.push(data);
                    current = Some(entry);
                }
                None => {
                    entry.text.push_str(body);
                    out.push(entry);
                }
            }
        }
    }

    pub fn login(&mut self, user: &str, password: &str) -> Result<(), String> {
        self.run(&format!("LOGIN {} {}", quote(user), quote(password))).map(drop).map_err(|e| format!("Gmail login failed: {e}"))
    }

    pub fn logout(mut self) {
        let _ = self.run("LOGOUT");
    }
}

/// `… {1234}` → (1234, the text before the brace).
fn literal_size(line: &str) -> Option<(usize, &str)> {
    let inner = line.strip_suffix('}')?;
    let open = inner.rfind('{')?;
    let size = inner[open + 1..].trim_end_matches('+').parse().ok()?;
    Some((size, &line[..open]))
}

/// The UIDs in `* SEARCH 1 2 3` responses.
pub fn search_results(responses: &[Untagged]) -> Vec<u32> {
    responses
        .iter()
        .filter_map(|r| r.text.strip_prefix("* SEARCH"))
        .flat_map(|rest| rest.split_whitespace().filter_map(|n| n.parse().ok()).collect::<Vec<u32>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_literals_and_numbers() {
        assert_eq!(literal_size("* 1 FETCH (UID 5 BODY[]<0> {1234}"), Some((1234, "* 1 FETCH (UID 5 BODY[]<0> ")));
        assert_eq!(literal_size("* 1 FETCH (UID 5)"), None);
        let r = Untagged { text: "* 3 FETCH (X-GM-THRID 1790000000000000001 UID 77 BODY[]<0> )".into(), literals: vec![] };
        assert_eq!(r.number("UID"), Some(77));
        assert_eq!(r.number("X-GM-THRID"), Some(1790000000000000001));
        assert_eq!(r.number("FLAGS"), None);
        let found = search_results(&[Untagged { text: "* SEARCH 4 9 12".into(), literals: vec![] }]);
        assert_eq!(found, [4, 9, 12]);
        assert_eq!(quote(r#"pa"ss\"#), r#""pa\"ss\\""#);
    }
}
