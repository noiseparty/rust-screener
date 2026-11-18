//! Protobuf `ItemsCombo` payloads: quote decoding and the raw field dump for probe mode.

use super::Update;
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) enum Val<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Other,
}

/// Iterates the (field number, value) pairs of one protobuf message. Yields `Err` once on malformed input.
pub(super) struct Fields<'a> {
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

pub(super) fn text_num(b: &[u8]) -> Option<f64> {
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

/// Every field of one message as the server sent it, keyed by protobuf field number and rendered
/// as text. Probe mode records these untouched, including fields the app does not use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawMsg {
    pub id: u64,
    /// BaseItem (field 1 of ItemsCombo).
    pub base: BTreeMap<u32, String>,
    /// SaleItem (field 2).
    pub sale: BTreeMap<u32, String>,
    /// StatsItem (field 3).
    pub stats: BTreeMap<u32, String>,
    /// Other top-level modules present: (field number, payload length).
    pub modules: Vec<(u32, usize)>,
}

pub(super) fn raw_text(v: &Val) -> String {
    match v {
        Val::Varint(n) => n.to_string(),
        Val::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) if !s.chars().any(char::is_control) => s.to_string(),
            _ => b.iter().map(|x| format!("{x:02x}")).collect(),
        },
        Val::Other => "<fixed>".to_string(),
    }
}

pub(super) fn raw_fields(buf: &[u8]) -> Option<BTreeMap<u32, String>> {
    let mut m = BTreeMap::new();
    for f in Fields::new(buf) {
        let (n, v) = f.ok()?;
        m.insert(n, raw_text(&v));
    }
    Some(m)
}

/// Like `decode_items_combo`, but keeps all raw fields as text. `None` for malformed payloads or when no id is known.
pub fn decode_raw(buf: &[u8], topic_id: Option<u64>) -> Option<RawMsg> {
    let mut m = RawMsg::default();
    for f in Fields::new(buf) {
        let (field, val) = f.ok()?;
        let Val::Bytes(sub) = val else { continue };
        match field {
            1 => m.base = raw_fields(sub)?,
            2 => m.sale = raw_fields(sub)?,
            3 => m.stats = raw_fields(sub)?,
            n => m.modules.push((n, sub.len())),
        }
    }
    m.id = m.base.get(&3).and_then(|v| v.parse().ok()).filter(|&v| v != 0).or(topic_id).unwrap_or(0);
    (m.id != 0).then_some(m)
}
