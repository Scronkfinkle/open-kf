//! The host-info query: a small UDP "what are you playing?" exchange on
//! the game port + 1 (Unreal's QueryPort: 7708 for KF's 7707), answered
//! by a host before anyone joins.
//!
//! A joining game asks it in `main.rs` before the game engine is built,
//! because the map is loaded at startup: with the answer, `--join ADDR`
//! alone is enough (the host's map, mode and length are used).
//!
//! The message is plain text, so more facts can be added later without
//! breaking older games:
//!
//! ```text
//! OPENKF-QUERY 1        <- fixed tag and format version
//! answer=info
//! map=KF-Farm
//! mode=Waves
//! ...
//! ```
//!
//! A reader ignores keys it does not know and uses a default for missing
//! ones; only a different version on the first line means "cannot read".
//! This socket is separate from the game connection (lightyear on the
//! game port), which is unchanged. std::net only (works on Windows too).

use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::runlog;

/// The first word of every query message.
pub const MAGIC: &str = "OPENKF-QUERY";
/// The format version (the number after MAGIC). Raise it only when old
/// games could no longer read the message; adding a key does not need it.
pub const VERSION: u32 = 1;
/// Requests are padded to this size, and the host answers only requests
/// at least this big and never with more bytes than it received, so the
/// port cannot be used to multiply someone else's traffic.
pub const REQUEST_SIZE: usize = 512;

/// What a host tells a joining game. Strings are as the host's log
/// writes them (`mode=Waves`, `length=Short`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostInfo {
    /// The host's netcode `PROTOCOL_ID`: a different one cannot connect.
    pub protocol: u64,
    /// The host's game port (lightyear connection).
    pub game_port: u16,
    pub map: String,
    pub mode: String,
    pub length: String,
    /// `difficulty=HellOnEarth` etc. (empty from an older host: the
    /// joiner keeps its own).
    pub difficulty: String,
    /// Players connected now (the host included).
    pub players: u32,
    pub max_players: u32,
    pub match_started: bool,
}

/// The query port that belongs to a game port.
pub fn query_port(game_port: u16) -> Option<u16> {
    game_port.checked_add(1)
}

fn header() -> String {
    format!("{MAGIC} {VERSION}\n")
}

/// A message's `key=value` lines.
type Pairs<'a> = Vec<(&'a str, &'a str)>;

/// Splits a message into its version and its `key=value` lines.
fn parse(bytes: &[u8]) -> Result<(u32, Pairs<'_>), String> {
    // Padding (zeros) is not part of the text.
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| "not text".to_string())?;
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let version = first
        .strip_prefix(MAGIC)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or_else(|| "not an Open KF query message".to_string())?;
    let pairs = lines.filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
    Ok((version, pairs))
}

/// A request for the host's info, padded to `REQUEST_SIZE`.
pub fn request_bytes() -> Vec<u8> {
    let mut b = format!("{}ask=info\n", header()).into_bytes();
    b.resize(REQUEST_SIZE, 0);
    b
}

/// Is this a request this host should answer?
pub fn is_request(bytes: &[u8]) -> bool {
    bytes.len() >= REQUEST_SIZE && matches!(parse(bytes), Ok((VERSION, pairs)) if pairs.contains(&("ask", "info")))
}

/// Values must stay on one line.
fn clean(v: &str) -> String {
    v.replace(['\n', '\r', '\0'], " ")
}

impl HostInfo {
    pub fn encode(&self) -> Vec<u8> {
        let mut s = header();
        s.push_str("answer=info\n");
        for (k, v) in [
            ("protocol", format!("{:#x}", self.protocol)),
            ("game_port", self.game_port.to_string()),
            ("map", clean(&self.map)),
            ("mode", clean(&self.mode)),
            ("length", clean(&self.length)),
            ("difficulty", clean(&self.difficulty)),
            ("players", self.players.to_string()),
            ("max_players", self.max_players.to_string()),
            ("match_started", self.match_started.to_string()),
        ] {
            s.push_str(&format!("{k}={v}\n"));
        }
        s.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let (version, pairs) = parse(bytes)?;
        if version != VERSION {
            return Err(format!("query format version {version}, this game reads {VERSION}"));
        }
        if !pairs.contains(&("answer", "info")) {
            return Err("not a host-info answer".into());
        }
        let mut info = HostInfo::default();
        for (k, v) in pairs {
            // Unknown keys (from a newer host) are skipped; a bad number
            // keeps the default.
            match k {
                "protocol" => info.protocol = u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0),
                "game_port" => info.game_port = v.parse().unwrap_or(0),
                "map" => info.map = v.to_string(),
                "mode" => info.mode = v.to_string(),
                "length" => info.length = v.to_string(),
                "difficulty" => info.difficulty = v.to_string(),
                "players" => info.players = v.parse().unwrap_or(0),
                "max_players" => info.max_players = v.parse().unwrap_or(0),
                "match_started" => info.match_started = v == "true",
                _ => {}
            }
        }
        if info.map.is_empty() {
            return Err("the answer has no map".into());
        }
        Ok(info)
    }
}

/// The host's current info, shared with the answering thread.
pub type SharedInfo = Arc<Mutex<HostInfo>>;

/// Host: opens the query socket on `bind` and answers requests from a
/// background thread (so it answers even while the map is loading). The
/// thread lives as long as the game. Returns the shared info (for the
/// game to update) and the address actually bound.
pub fn start_responder(bind: SocketAddr, info: HostInfo) -> std::io::Result<(SharedInfo, SocketAddr)> {
    let socket = UdpSocket::bind(bind)?;
    let local = socket.local_addr()?;
    let shared: SharedInfo = Arc::new(Mutex::new(info));
    let theirs = shared.clone();
    std::thread::Builder::new().name("net-query".into()).spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            let (n, from) = match socket.recv_from(&mut buf) {
                Ok(r) => r,
                // Windows reports an earlier answer that could not be
                // delivered as an error on the next receive: keep going
                // (after a pause, so a lasting error cannot spin).
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };
            if !is_request(&buf[..n]) {
                runlog::kv("net_query_ignored", &format!("from={from} bytes={n}"));
                continue;
            }
            let (answer, info) = {
                let i = theirs.lock().unwrap_or_else(|e| e.into_inner());
                (i.encode(), i.clone())
            };
            if answer.len() > n {
                runlog::kv("net_query_ignored", &format!("from={from} bytes={n} reason=answer_bigger_than_request answer_bytes={}", answer.len()));
                continue;
            }
            let sent = socket.send_to(&answer, from);
            runlog::kv(
                "net_query_answered",
                &format!("from={from} bytes={} ok={} map={} mode={} length={} players={} match_started={}", answer.len(), sent.is_ok(), info.map, info.mode, info.length, info.players, info.match_started),
            );
        }
    })?;
    Ok((shared, local))
}

/// Joiner: asks the host at `addr` (its query port) for its info. Tries
/// `tries` times, waiting up to `wait` for each answer. Returns the
/// info and how long it took.
pub fn ask(addr: SocketAddr, tries: u32, wait: Duration) -> Result<(HostInfo, Duration), String> {
    let bind: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let socket = UdpSocket::bind(bind).map_err(|e| format!("cannot open a UDP socket: {e}"))?;
    let start = Instant::now();
    let request = request_bytes();
    let mut last_problem = "no answer".to_string();
    let mut buf = [0u8; 2048];
    for attempt in 1..=tries.max(1) {
        let try_start = Instant::now();
        if let Err(e) = socket.send_to(&request, addr) {
            last_problem = format!("cannot send: {e}");
        } else {
            runlog::kv("net_query_sent", &format!("to={addr} attempt={attempt} bytes={}", request.len()));
        }
        // Read until this try's time is up (other packets are skipped).
        loop {
            let left = wait.saturating_sub(try_start.elapsed());
            if left.is_zero() {
                break;
            }
            let _ = socket.set_read_timeout(Some(left));
            match socket.recv_from(&mut buf) {
                Ok((n, from)) => match HostInfo::decode(&buf[..n]) {
                    Ok(info) => return Ok((info, start.elapsed())),
                    Err(e) => last_problem = format!("unreadable answer from {from}: {e}"),
                },
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                // "Connection refused" (nothing listens there) comes back
                // at once; wait out the try so a host still starting up
                // gets its chance.
                Err(e) => {
                    last_problem = format!("{e}");
                    std::thread::sleep(wait.saturating_sub(try_start.elapsed()));
                    break;
                }
            }
        }
    }
    Err(last_problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HostInfo {
        HostInfo { protocol: 0x4F4B_4600_0007, game_port: 7707, map: "KF-Farm".into(), mode: "Waves".into(), length: "Short".into(), difficulty: "HellOnEarth".into(), players: 2, max_players: 6, match_started: true }
    }

    #[test]
    fn info_round_trip() {
        assert_eq!(HostInfo::decode(&sample().encode()).unwrap(), sample());
        assert_eq!(query_port(7707), Some(7708));
        assert_eq!(query_port(u16::MAX), None);
    }

    #[test]
    fn unknown_keys_skipped_missing_keys_default() {
        let msg = format!("{MAGIC} {VERSION}\nanswer=info\nmap=KF-Manor\nfuture_thing=42\nweird line\n");
        let i = HostInfo::decode(msg.as_bytes()).unwrap();
        assert_eq!(i.map, "KF-Manor");
        assert_eq!(i.mode, "");
        assert_eq!(i.players, 0);
    }

    #[test]
    fn garbage_and_other_versions_refused() {
        assert!(HostInfo::decode(b"hello").is_err());
        assert!(HostInfo::decode(&[0xff, 0xfe, 0x00]).is_err());
        assert!(HostInfo::decode(format!("{MAGIC} 99\nanswer=info\nmap=KF-Farm\n").as_bytes()).is_err());
        // A request is not an answer, and an answer is not a request.
        assert!(HostInfo::decode(&request_bytes()).is_err());
        assert!(!is_request(&sample().encode()));
    }

    #[test]
    fn requests_must_be_padded() {
        let r = request_bytes();
        assert_eq!(r.len(), REQUEST_SIZE);
        assert!(is_request(&r));
        assert!(!is_request(&r[..40]));
        assert!(sample().encode().len() < REQUEST_SIZE);
    }

    #[test]
    fn values_stay_on_one_line() {
        let mut i = sample();
        i.map = "KF-Bad\nmode=Debug".into();
        assert_eq!(HostInfo::decode(&i.encode()).unwrap().mode, "Waves");
    }

    #[test]
    fn loopback_ask_and_answer() {
        let (shared, local) = start_responder("127.0.0.1:0".parse().unwrap(), sample()).unwrap();
        let (got, _) = ask(local, 3, Duration::from_millis(500)).unwrap();
        assert_eq!(got, sample());
        // The game updates the shared info; the next answer has it.
        shared.lock().unwrap().players = 3;
        assert_eq!(ask(local, 3, Duration::from_millis(500)).unwrap().0.players, 3);
    }

    #[test]
    fn no_host_fails_quickly() {
        // Bind and drop a socket to find a port nobody listens on.
        let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let t = Instant::now();
        assert!(ask(port, 2, Duration::from_millis(200)).is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
