//! Webull push stream client: MQTT 3.1.1 over WSS, protobuf `ItemsCombo` payloads.
//!
//! A single worker thread owns the connection. The app sends it the wanted set of
//! tickerIds (`Cmd::SetTickers`); it sends back decoded quotes in 250 ms batches
//! (`Msg::Quotes`) and a status snapshot about once a second (`Msg::Stream`).
//! Anonymous access (user/pass `test`/`test`) only delivers type 102 with flag "1".

use crate::app::Msg;
use crate::scraper::USER_AGENT;
use crate::session::Session;
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
#[allow(dead_code)] // high/low/status are consumed by later steps (series store, highs/lows, status).
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

// ───────────────────────────── MQTT 3.1.1 ─────────────────────────────

#[derive(Debug, PartialEq)]
pub enum Packet {
    ConnAck { code: u8 },
    SubAck { id: u16 },
    Publish { topic: String, payload: Vec<u8> },
    PingResp,
    Other(u8),
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn put_remaining(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

fn frame(first: u8, body: Vec<u8>) -> Vec<u8> {
    let mut out = vec![first];
    put_remaining(&mut out, body.len());
    out.extend(body);
    out
}

pub fn encode_connect(client_id: &str, user: &str, pass: &str, keepalive: u16) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, "MQTT");
    b.push(4); // protocol level 3.1.1
    b.push(0xC2); // username, password, clean session
    b.extend_from_slice(&keepalive.to_be_bytes());
    put_str(&mut b, client_id);
    put_str(&mut b, user);
    put_str(&mut b, pass);
    frame(0x10, b)
}

pub fn encode_subscribe(id: u16, topic: &str) -> Vec<u8> {
    let mut b = id.to_be_bytes().to_vec();
    put_str(&mut b, topic);
    b.push(0); // QoS 0
    frame(0x82, b)
}

pub fn encode_unsubscribe(id: u16, topic: &str) -> Vec<u8> {
    let mut b = id.to_be_bytes().to_vec();
    put_str(&mut b, topic);
    frame(0xA2, b)
}

pub fn encode_pingreq() -> Vec<u8> {
    vec![0xC0, 0x00]
}

/// Removes every complete packet from the front of `buf` and returns them.
/// A trailing partial packet is left in `buf` for the next call.
pub fn parse_packets(buf: &mut Vec<u8>) -> Result<Vec<Packet>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let rest = &buf[pos..];
        if rest.len() < 2 {
            break;
        }
        let mut len = 0usize;
        let mut mult = 1usize;
        let mut i = 1;
        let header = loop {
            let Some(&b) = rest.get(i) else { break None };
            len += (b & 0x7F) as usize * mult;
            i += 1;
            if b & 0x80 == 0 {
                break Some(i);
            }
            mult *= 128;
            if mult > 128 * 128 * 128 {
                return Err("malformed remaining length".into());
            }
        };
        let Some(header) = header else { break };
        if rest.len() < header + len {
            break;
        }
        let first = rest[0];
        out.push(parse_packet(first, &rest[header..header + len])?);
        pos += header + len;
    }
    buf.drain(..pos);
    Ok(out)
}

fn parse_packet(first: u8, body: &[u8]) -> Result<Packet, String> {
    let short = || "truncated packet".to_string();
    let u16_at = |i: usize| -> Result<u16, String> {
        Ok(u16::from_be_bytes([*body.get(i).ok_or_else(short)?, *body.get(i + 1).ok_or_else(short)?]))
    };
    Ok(match first >> 4 {
        2 => Packet::ConnAck { code: *body.get(1).ok_or_else(short)? },
        9 => Packet::SubAck { id: u16_at(0)? },
        13 => Packet::PingResp,
        3 => {
            let qos = (first >> 1) & 3;
            let tlen = u16_at(0)? as usize;
            let topic = body.get(2..2 + tlen).ok_or_else(short)?;
            let start = 2 + tlen + if qos > 0 { 2 } else { 0 };
            Packet::Publish {
                topic: String::from_utf8_lossy(topic).into_owned(),
                payload: body.get(start..).ok_or_else(short)?.to_vec(),
            }
        }
        t => Packet::Other(t),
    })
}

fn header_topic(did: &str) -> String {
    format!(
        r#"{{"header":{{"did":"{did}","hl":"en","app":"global","os":"web","osv":"h5","ver":"1.0.0","platform":"webull-us"}}}}"#
    )
}

fn tickers_topic(ids: &[u64]) -> String {
    let list: Vec<String> = ids.iter().map(u64::to_string).collect();
    format!(r#"{{"tickerIds":[{}],"type":"102","flag":"1"}}"#, list.join(","))
}

// ───────────────────────────── protobuf ─────────────────────────────

enum Val<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Other,
}

/// Iterates the (field number, value) pairs of one protobuf message. Yields `Err` once on malformed input.
struct Fields<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Fields<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Fields { buf, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            v |= ((b & 0x7F) as u64).checked_shl(shift).unwrap_or(0);
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len())?;
        self.pos = end;
        Some(())
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<(u32, Val<'a>), ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let item = (|| {
            let key = self.varint()?;
            let field = (key >> 3) as u32;
            let val = match key & 7 {
                0 => Val::Varint(self.varint()?),
                1 => self.skip(8).map(|_| Val::Other)?,
                5 => self.skip(4).map(|_| Val::Other)?,
                2 => {
                    let n = self.varint()? as usize;
                    let start = self.pos;
                    self.skip(n)?;
                    Val::Bytes(&self.buf[start..start + n])
                }
                _ => return None,
            };
            Some((field, val))
        })();
        match item {
            Some(x) => Some(Ok(x)),
            None => {
                self.pos = self.buf.len();
                Some(Err(()))
            }
        }
    }
}

fn text_num(b: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(b).ok()?.replace(',', "");
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Decodes a PUBLISH payload (`ItemsCombo`). `topic_id` is the tickerId from the topic JSON,
/// used when the payload's BaseItem lacks one. Returns `None` for malformed payloads or when no id is known.
pub fn decode_items_combo(buf: &[u8], topic_id: Option<u64>) -> Option<Update> {
    let mut u = Update {
        id: topic_id.unwrap_or(0),
        trade_ms: 0,
        seq: 0,
        close: None,
        p_price: None,
        change_ratio: None,
        volume: None,
        mcap: None,
        high: None,
        low: None,
        status: String::new(),
        recv: Instant::now(),
    };
    for f in Fields::new(buf) {
        let (field, val) = f.ok()?;
        let Val::Bytes(sub) = val else { continue };
        match field {
            1 => {
                for f in Fields::new(sub) {
                    match f.ok()? {
                        (2, Val::Varint(v)) => u.seq = v,
                        (3, Val::Varint(v)) if v != 0 => u.id = v,
                        (4, Val::Bytes(b)) => u.status = String::from_utf8_lossy(b).into_owned(),
                        (5, Val::Varint(v)) => u.trade_ms = v as i64,
                        _ => {}
                    }
                }
            }
            2 => {
                for f in Fields::new(sub) {
                    match f.ok()? {
                        (1, Val::Bytes(b)) => u.close = text_num(b),
                        (3, Val::Bytes(b)) => u.change_ratio = text_num(b),
                        (4, Val::Bytes(b)) => u.p_price = text_num(b),
                        (7, Val::Bytes(b)) => u.mcap = text_num(b),
                        _ => {}
                    }
                }
            }
            3 => {
                for f in Fields::new(sub) {
                    match f.ok()? {
                        (3, Val::Bytes(b)) => u.volume = text_num(b).filter(|v| *v >= 0.0).map(|v| v.round() as u64),
                        (9, Val::Bytes(b)) => u.high = text_num(b),
                        (10, Val::Bytes(b)) => u.low = text_num(b),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (u.id != 0).then_some(u)
}

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
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                Ok(Vec::new())
            }
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
    wanted: BTreeSet<u64>,
    st: StreamStatus,
    did: String,
}

impl Shared {
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
    thread::spawn(move || {
        let mut sh = Shared { tx, rx, wanted: BTreeSet::new(), st: StreamStatus::default(), did: random_did() };
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
        let err = match run_connection(sh, &tls, &mut got_data) {
            Ok(()) => return, // app gone
            Err(e) => e,
        };
        sh.st.connected = false;
        sh.st.subs = 0;
        sh.st.error = Some(err);
        if got_data {
            backoff = Duration::from_secs(1);
        }
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
    c.wait_for("SUBACK", CONNECT_TIMEOUT, |p| *p == Packet::SubAck { id })?;

    sh.st.connected = true;
    sh.st.connections += 1;
    sh.st.error = None;
    sh.st.last_msg = None;
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
        }
        for chunk in add.chunks(SUB_CHUNK) {
            let id = c.id();
            c.send(encode_subscribe(id, &tickers_topic(chunk)))?;
            subscribed.extend(chunk);
        }
        sh.st.subs = subscribed.len();

        for p in c.poll()? {
            if let Packet::Publish { topic, payload } = p {
                if let Some(u) = decode_items_combo(&payload, topic_ticker_id(&topic)) {
                    count += 1;
                    *got_data = true;
                    sh.st.last_msg = Some(u.recv);
                    batch.push(u);
                }
            }
        }

        let now = Instant::now();
        if now.duration_since(last_batch) >= BATCH {
            last_batch = now;
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- MQTT ----

    #[test]
    fn connect_packet_bytes() {
        let p = encode_connect("ab", "u", "p", 30);
        let expect: Vec<u8> = vec![
            0x10, 0x14, // CONNECT, remaining 20
            0, 4, b'M', b'Q', b'T', b'T', 4, 0xC2, 0, 30, // protocol, level, flags, keepalive
            0, 2, b'a', b'b', // client id
            0, 1, b'u', // user
            0, 1, b'p', // password
        ];
        assert_eq!(p, expect);
    }

    #[test]
    fn subscribe_unsubscribe_ping_bytes() {
        assert_eq!(encode_subscribe(1, "hi"), vec![0x82, 7, 0, 1, 0, 2, b'h', b'i', 0]);
        assert_eq!(encode_unsubscribe(258, "x"), vec![0xA2, 5, 1, 2, 0, 1, b'x']);
        assert_eq!(encode_pingreq(), vec![0xC0, 0]);
    }

    #[test]
    fn remaining_length_multi_byte() {
        let topic = "t".repeat(200);
        let p = encode_subscribe(1, &topic);
        // body = 2 + 2 + 200 + 1 = 205 -> 0xCD 0x01
        assert_eq!(&p[..3], &[0x82, 0xCD, 0x01]);
        assert_eq!(p.len(), 3 + 205);
    }

    #[test]
    fn parses_connack_suback_pingresp() {
        let mut buf = vec![0x20, 2, 0, 0, 0x90, 3, 0, 7, 0, 0xD0, 0];
        let pk = parse_packets(&mut buf).unwrap();
        assert_eq!(pk, vec![Packet::ConnAck { code: 0 }, Packet::SubAck { id: 7 }, Packet::PingResp]);
        assert!(buf.is_empty());
    }

    #[test]
    fn parses_publish_qos0_and_qos1() {
        let mut buf = vec![0x30, 7, 0, 2, b't', b'p', 1, 2, 3];
        assert_eq!(
            parse_packets(&mut buf).unwrap(),
            vec![Packet::Publish { topic: "tp".into(), payload: vec![1, 2, 3] }]
        );
        // QoS 1 carries a packet id after the topic.
        let mut buf = vec![0x32, 8, 0, 2, b't', b'p', 0, 9, 4, 5];
        assert_eq!(
            parse_packets(&mut buf).unwrap(),
            vec![Packet::Publish { topic: "tp".into(), payload: vec![4, 5] }]
        );
    }

    #[test]
    fn partial_packets_stay_buffered() {
        let mut buf = vec![0x30, 7, 0, 2, b't'];
        assert!(parse_packets(&mut buf).unwrap().is_empty());
        assert_eq!(buf.len(), 5);
        buf.extend_from_slice(&[b'p', 1, 2, 3, 0xD0, 0]);
        let pk = parse_packets(&mut buf).unwrap();
        assert_eq!(pk.len(), 2);
        assert!(buf.is_empty());
        // length header itself incomplete
        let mut buf = vec![0x30, 0x80];
        assert!(parse_packets(&mut buf).unwrap().is_empty());
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn rejects_bad_input() {
        let mut buf = vec![0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
        assert!(parse_packets(&mut buf).is_err());
        let mut buf = vec![0x20, 1, 0]; // CONNACK too short
        assert!(parse_packets(&mut buf).is_err());
    }

    #[test]
    fn topic_strings() {
        assert_eq!(tickers_topic(&[1, 22]), r#"{"tickerIds":[1,22],"type":"102","flag":"1"}"#);
        assert!(header_topic("abc").starts_with(r#"{"header":{"did":"abc","hl":"en""#));
        assert_eq!(topic_ticker_id(r#"{"type":102,"tickerId":950978197,"modules":[""],"flag":"1"}"#), Some(950978197));
        assert_eq!(topic_ticker_id("garbage"), None);
    }

    // ---- protobuf ----

    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }
    fn f_varint(field: u32, v: u64) -> Vec<u8> {
        [varint((field as u64) << 3), varint(v)].concat()
    }
    fn f_bytes(field: u32, b: &[u8]) -> Vec<u8> {
        [varint(((field as u64) << 3) | 2), varint(b.len() as u64), b.to_vec()].concat()
    }
    fn f_str(field: u32, s: &str) -> Vec<u8> {
        f_bytes(field, s.as_bytes())
    }

    fn sample() -> Vec<u8> {
        let base = [f_varint(1, 5), f_varint(2, 77), f_varint(3, 950978197), f_str(4, "T"), f_varint(5, 1_763_400_000_123), f_varint(8, 9)].concat();
        let sale = [f_str(1, "1.25"), f_str(2, "0.25"), f_str(3, "0.25"), f_str(4, "1.30"), f_str(7, "12,500,000")].concat();
        let stats = [f_str(1, "1763400000"), f_str(3, "1234567"), f_str(8, "1.0"), f_str(9, "1.4"), f_str(10, "0.9"), f_varint(52, 3)].concat();
        [f_bytes(1, &base), f_bytes(2, &sale), f_bytes(3, &stats), f_bytes(6, b"ignored")].concat()
    }

    #[test]
    fn decodes_items_combo() {
        let u = decode_items_combo(&sample(), None).unwrap();
        assert_eq!(u.id, 950978197);
        assert_eq!(u.seq, 77);
        assert_eq!(u.trade_ms, 1_763_400_000_123);
        assert_eq!(u.status, "T");
        assert_eq!(u.close, Some(1.25));
        assert_eq!(u.p_price, Some(1.30));
        assert_eq!(u.change_ratio, Some(0.25));
        assert_eq!(u.mcap, Some(12_500_000.0));
        assert_eq!(u.volume, Some(1_234_567));
        assert_eq!(u.high, Some(1.4));
        assert_eq!(u.low, Some(0.9));
    }

    #[test]
    fn missing_and_bad_numbers_are_none() {
        let sale = [f_str(1, ""), f_str(4, "n/a")].concat();
        let stats = f_str(3, "-5");
        let buf = [f_bytes(1, &f_varint(3, 42)), f_bytes(2, &sale), f_bytes(3, &stats)].concat();
        let u = decode_items_combo(&buf, None).unwrap();
        assert_eq!(u.id, 42);
        assert_eq!((u.close, u.p_price, u.volume), (None, None, None));
    }

    #[test]
    fn id_falls_back_to_topic_and_is_required() {
        let buf = f_bytes(3, &f_str(3, "10"));
        assert_eq!(decode_items_combo(&buf, Some(7)).unwrap().id, 7);
        assert!(decode_items_combo(&buf, None).is_none());
    }

    #[test]
    fn truncated_or_garbage_protobuf_is_rejected() {
        let mut b = sample();
        b.truncate(b.len() - 3);
        assert!(decode_items_combo(&b, Some(1)).is_none());
        assert!(decode_items_combo(&[0x0B], Some(1)).is_none()); // group wire type
        assert!(decode_items_combo(&[0x80; 12], Some(1)).is_none()); // endless varint
    }

    #[test]
    fn unknown_wire_types_are_skipped() {
        // fixed64 (field 9) and fixed32 (field 10) at top level, then a valid base
        let mut b = vec![(9 << 3) | 1];
        b.extend([0u8; 8]);
        b.push((10 << 3) | 5);
        b.extend([0u8; 4]);
        b.extend(f_bytes(1, &f_varint(3, 5)));
        assert_eq!(decode_items_combo(&b, None).unwrap().id, 5);
    }
}
