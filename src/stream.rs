//! Webull push stream client: MQTT 3.1.1 over WSS, protobuf `ItemsCombo` payloads.
//!
//! A single worker thread owns the connection. The app sends it the wanted set of
//! tickerIds (`Cmd::SetTickers`); it sends back decoded quotes in 250 ms batches
//! (`Msg::Quotes`) and a status snapshot about once a second (`Msg::Stream`).
//! Anonymous access (user/pass `test`/`test`) only delivers type 102 with flag "1".

use crate::app::Msg;
use crate::net::USER_AGENT;
use crate::session::Session;
use crate::trace::{tlog, tnote};
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::Message;

const HOST: &str = "wspush.webullfintech.com";
const URL: &str = "wss://wspush.webullfintech.com:443/mqtt";
const KEEPALIVE_SECS: u16 = 30;
const PING_EVERY: Duration = Duration::from_secs(15);
const BATCH: Duration = Duration::from_millis(250);
const STATUS_EVERY: Duration = Duration::from_secs(1);
/// A connected stream that delivers nothing for this long (outside Closed) is torn down and redialled.
const STALE: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(200);
const SUB_CHUNK: usize = 100;
const BACKOFF_MAX: Duration = Duration::from_secs(30);

pub enum Cmd {
    /// The complete set of tickerIds to be subscribed. The worker diffs it against what it holds.
    SetTickers(Vec<u64>),
}

/// One decoded quote message. Numbers the server sent as strings are parsed; absent or
/// unparseable values are `None`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // high/low/status are decoded (and tested) but nothing reads them yet
pub struct Update {
    pub id: u64,
    /// Exchange timestamp in ms (0 if absent).
    pub trade_ms: i64,
    pub seq: u64,
    pub close: Option<f64>,
    pub p_price: Option<f64>,
    pub change_ratio: Option<f64>,
    /// Cumulative day volume.
    pub volume: Option<u64>,
    pub mcap: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub status: String,
    pub recv: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct StreamStatus {
    pub connected: bool,
    /// Number of connections established so far; a change means a reconnect.
    pub connections: u64,
    pub subs: usize,
    pub msgs_per_sec: f64,
    pub last_msg: Option<Instant>,
    /// Seconds until the next reconnect attempt, while waiting out a backoff.
    pub retry_in: Option<u64>,
    pub error: Option<String>,
}

mod mqtt;
mod proto;
#[cfg(test)]
mod tests;

use mqtt::*;
pub use proto::RawMsg;
use proto::*;

fn header_topic(did: &str) -> String {
    format!(r#"{{"header":{{"did":"{did}","hl":"en","app":"global","os":"web","osv":"h5","ver":"1.0.0","platform":"webull-us"}}}}"#)
}

fn tickers_topic(ids: &[u64]) -> String {
    let list: Vec<String> = ids.iter().map(u64::to_string).collect();
    format!(r#"{{"tickerIds":[{}],"type":"102","flag":"1"}}"#, list.join(","))
}

/// Diagnostics for probe mode, sent with the wall-clock time they happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// WebSocket, CONNECT and the header subscription all succeeded; `n` counts connections so far.
    Connected {
        n: u64,
    },
    /// A SUBSCRIBE (or UNSUBSCRIBE) packet was sent for `count` tickers; `total` is the number held afterwards.
    Subscribe {
        id: u16,
        count: usize,
        total: usize,
    },
    Unsubscribe {
        id: u16,
        count: usize,
        total: usize,
    },
    /// Return codes of a SUBACK (0 = QoS 0 granted, 0x80 = refused).
    SubAck {
        id: u16,
        codes: Vec<u8>,
    },
    Disconnected {
        reason: String,
    },
    Undecodable {
        topic: String,
        len: usize,
    },
    Raw(RawMsg),
}

pub type Stamped = (DateTime<Utc>, Event);

fn topic_ticker_id(topic: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(topic).ok()?;
    let id = v.get("tickerId")?;
    id.as_u64().or_else(|| id.as_str()?.parse().ok())
}

// ───────────────────────────── connection ─────────────────────────────

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;
type Ws = tungstenite::WebSocket<Tls>;

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

fn dial(cfg: Arc<rustls::ClientConfig>) -> Result<Ws, String> {
    let addr = (HOST, 443).to_socket_addrs().map_err(|e| format!("dns: {e}"))?.next().ok_or("dns: no address")?;
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| format!("tcp: {e}"))?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(|e| e.to_string())?;
    let name = rustls::pki_types::ServerName::try_from(HOST).map_err(|e| e.to_string())?;
    let conn = rustls::ClientConnection::new(cfg, name).map_err(|e| e.to_string())?;
    let tls = rustls::StreamOwned::new(conn, tcp);

    let mut req = URL.into_client_request().map_err(|e| e.to_string())?;
    let h = req.headers_mut();
    h.insert("Sec-WebSocket-Protocol", HeaderValue::from_static("mqtt"));
    h.insert("Origin", HeaderValue::from_static("https://webull.com"));
    h.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
    let (ws, _) = tungstenite::client::client(req, tls).map_err(|e| format!("websocket: {e}"))?;
    ws.get_ref().sock.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;
    Ok(ws)
}

struct Conn {
    ws: Ws,
    buf: Vec<u8>,
    next_id: u16,
}

impl Conn {
    fn send(&mut self, data: Vec<u8>) -> Result<(), String> {
        self.ws.send(Message::Binary(data.into())).map_err(|e| format!("send: {e}"))
    }

    fn id(&mut self) -> u16 {
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.next_id
    }

    /// Waits up to one poll interval for a WebSocket message and returns the MQTT packets in it.
    fn poll(&mut self) -> Result<Vec<Packet>, String> {
        match self.ws.read() {
            Ok(Message::Binary(b)) => {
                self.buf.extend_from_slice(&b);
                parse_packets(&mut self.buf)
            }
            Ok(Message::Close(_)) => Err("server closed the connection".into()),
            Ok(_) => Ok(Vec::new()),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Ok(Vec::new()),
            Err(e) => Err(format!("read: {e}")),
        }
    }

    /// Reads until `want` accepts a packet or `timeout` passes. Other packets are dropped.
    fn wait_for(&mut self, what: &str, timeout: Duration, want: impl Fn(&Packet) -> bool) -> Result<Packet, String> {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if let Some(p) = self.poll()?.into_iter().find(&want) {
                return Ok(p);
            }
        }
        Err(format!("timed out waiting for {what}"))
    }
}

struct Shared {
    tx: Sender<Msg>,
    rx: Receiver<Cmd>,
    /// Probe mode only: receives raw messages and connection events.
    events: Option<Sender<Stamped>>,
    wanted: BTreeSet<u64>,
    st: StreamStatus,
    did: String,
}

impl Shared {
    fn event(&self, e: impl FnOnce() -> Event) {
        if let Some(tx) = &self.events {
            let _ = tx.send((Utc::now(), e()));
        }
    }

    fn emit(&self) -> bool {
        self.tx.send(Msg::Stream(self.st.clone())).is_ok()
    }

    /// Applies pending commands. Returns false when the app side has gone away.
    fn drain(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(Cmd::SetTickers(ids)) => self.wanted = ids.into_iter().collect(),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }
}

fn random_did() -> String {
    let rs = std::collections::hash_map::RandomState::new();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let part = |salt: u64| {
        let mut h = rs.build_hasher();
        h.write_u128(nanos);
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}

pub fn spawn(tx: Sender<Msg>, rx: Receiver<Cmd>) {
    spawn_with(tx, rx, None);
}

/// As `spawn`, and with `events` set the worker also reports raw messages and connection events (probe mode).
pub fn spawn_with(tx: Sender<Msg>, rx: Receiver<Cmd>, events: Option<Sender<Stamped>>) {
    thread::spawn(move || {
        let mut sh = Shared { tx, rx, events, wanted: BTreeSet::new(), st: StreamStatus::default(), did: random_did() };
        supervise(&mut sh);
    });
}

/// Connect, run, back off, repeat. Returns when the app side is gone.
fn supervise(sh: &mut Shared) {
    let tls = match tls_config() {
        Ok(c) => c,
        Err(e) => {
            sh.st.error = Some(format!("tls: {e}"));
            sh.emit();
            return;
        }
    };
    let mut backoff = Duration::from_secs(1);
    loop {
        // Idle until there is something to subscribe to and the market is not closed.
        while sh.wanted.is_empty() || Session::now() == Session::Closed {
            if !wait(sh, Duration::from_secs(1), false) {
                return;
            }
        }
        let mut got_data = false;
        tlog!("stream", "dialling for {} tickers", sh.wanted.len());
        let err = match run_connection(sh, &tls, &mut got_data) {
            Ok(()) => return, // app gone
            Err(e) => e,
        };
        sh.st.connected = false;
        sh.st.subs = 0;
        sh.event(|| Event::Disconnected { reason: err.clone() });
        sh.st.error = Some(err);
        if got_data {
            backoff = Duration::from_secs(1);
        }
        tlog!("stream", "disconnected: {} (retry in {} s)", sh.st.error.as_deref().unwrap_or(""), backoff.as_secs());
        if !wait(sh, backoff, true) {
            return;
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Sleeps for `dur`, applying commands and emitting status (with a retry countdown when `retry` is set).
/// Returns false when the app side is gone.
fn wait(sh: &mut Shared, dur: Duration, retry: bool) -> bool {
    let end = Instant::now() + dur;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        sh.st.retry_in = retry.then(|| left.as_secs() + 1);
        if !sh.emit() {
            return false;
        }
        if left.is_zero() {
            sh.st.retry_in = None;
            return true;
        }
        match sh.rx.recv_timeout(left.min(Duration::from_secs(1))) {
            Ok(Cmd::SetTickers(ids)) => sh.wanted = ids.into_iter().collect(),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Runs one connection until it fails. `Ok(())` means the app side went away.
fn run_connection(sh: &mut Shared, tls: &Arc<rustls::ClientConfig>, got_data: &mut bool) -> Result<(), String> {
    let mut c = Conn { ws: dial(tls.clone())?, buf: Vec::new(), next_id: 0 };
    c.send(encode_connect(&sh.did, "test", "test", KEEPALIVE_SECS))?;
    match c.wait_for("CONNACK", CONNECT_TIMEOUT, |p| matches!(p, Packet::ConnAck { .. }))? {
        Packet::ConnAck { code: 0 } => {}
        Packet::ConnAck { code } => return Err(format!("connection refused (code {code})")),
        _ => unreachable!(),
    }
    // The header topic has to be subscribed before any ticker topic.
    let id = c.id();
    c.send(encode_subscribe(id, &header_topic(&sh.did)))?;
    if let Packet::SubAck { codes, .. } =
        c.wait_for("SUBACK", CONNECT_TIMEOUT, |p| matches!(p, Packet::SubAck { id: i, .. } if *i == id))?
    {
        sh.event(|| Event::SubAck { id, codes });
    }

    sh.st.connected = true;
    sh.st.connections += 1;
    sh.st.error = None;
    sh.st.last_msg = None;
    sh.event(|| Event::Connected { n: sh.st.connections });
    tlog!("stream", "connected #{}", sh.st.connections);
    // Sent before any quote of this connection so the app can drop state from the previous one.
    if !sh.emit() {
        return Ok(());
    }
    let mut subscribed: HashSet<u64> = HashSet::new();
    let mut batch: Vec<Update> = Vec::new();
    let mut count = 0u32;
    let started = Instant::now();
    let (mut last_batch, mut last_status, mut last_ping, mut window_start) = (started, started, started, started);

    loop {
        if !sh.drain() {
            return Ok(());
        }
        // Reconcile the subscription set.
        let add: Vec<u64> = sh.wanted.iter().copied().filter(|i| !subscribed.contains(i)).collect();
        let del: Vec<u64> = subscribed.iter().copied().filter(|i| !sh.wanted.contains(i)).collect();
        for chunk in del.chunks(SUB_CHUNK) {
            let id = c.id();
            c.send(encode_unsubscribe(id, &tickers_topic(chunk)))?;
            for i in chunk {
                subscribed.remove(i);
            }
            sh.event(|| Event::Unsubscribe { id, count: chunk.len(), total: subscribed.len() });
        }
        for chunk in add.chunks(SUB_CHUNK) {
            let id = c.id();
            c.send(encode_subscribe(id, &tickers_topic(chunk)))?;
            subscribed.extend(chunk);
            sh.event(|| Event::Subscribe { id, count: chunk.len(), total: subscribed.len() });
        }
        if !add.is_empty() || !del.is_empty() {
            tlog!("stream.subs", "+{} -{}, {} held", add.len(), del.len(), subscribed.len());
        }
        sh.st.subs = subscribed.len();

        for p in c.poll()? {
            match p {
                Packet::Publish { topic, payload } => {
                    let tid = topic_ticker_id(&topic);
                    if sh.events.is_some() {
                        match decode_raw(&payload, tid) {
                            Some(raw) => sh.event(|| Event::Raw(raw)),
                            None => sh.event(|| Event::Undecodable { topic: topic.clone(), len: payload.len() }),
                        }
                    }
                    if let Some(u) = decode_items_combo(&payload, tid) {
                        count += 1;
                        *got_data = true;
                        sh.st.last_msg = Some(u.recv);
                        batch.push(u);
                    }
                }
                Packet::SubAck { id, codes } => sh.event(|| Event::SubAck { id, codes }),
                _ => {}
            }
        }

        let now = Instant::now();
        if now.duration_since(last_batch) >= BATCH {
            last_batch = now;
            if !batch.is_empty() {
                tnote!("stream.batch", "{} quotes", batch.len());
            }
            if !batch.is_empty() && sh.tx.send(Msg::Quotes(std::mem::take(&mut batch))).is_err() {
                return Ok(());
            }
        }
        if now.duration_since(last_ping) >= PING_EVERY {
            last_ping = now;
            c.send(encode_pingreq())?;
        }
        if now.duration_since(last_status) >= STATUS_EVERY {
            sh.st.msgs_per_sec = count as f64 / now.duration_since(window_start).as_secs_f64().max(0.001);
            tnote!("stream.rate", "{count} quotes/s, {} subs", subscribed.len());
            count = 0;
            window_start = now;
            last_status = now;
            if !sh.emit() {
                return Ok(());
            }
        }
        let silent_since = sh.st.last_msg.unwrap_or(started);
        if !subscribed.is_empty() && now.duration_since(silent_since) > STALE && Session::now() != Session::Closed {
            return Err("no data from stream".into());
        }
    }
}
