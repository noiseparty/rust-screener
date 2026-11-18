//! MQTT 3.1.1 packets: the few the client sends and the parser for what comes back.

#[derive(Debug, PartialEq)]
pub enum Packet {
    ConnAck { code: u8 },
    SubAck { id: u16, codes: Vec<u8> },
    Publish { topic: String, payload: Vec<u8> },
    PingResp,
    Other(u8),
}

pub(super) fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

pub(super) fn put_remaining(out: &mut Vec<u8>, mut n: usize) {
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

pub(super) fn frame(first: u8, body: Vec<u8>) -> Vec<u8> {
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

pub(super) fn parse_packet(first: u8, body: &[u8]) -> Result<Packet, String> {
    let short = || "truncated packet".to_string();
    let u16_at = |i: usize| -> Result<u16, String> {
        Ok(u16::from_be_bytes([*body.get(i).ok_or_else(short)?, *body.get(i + 1).ok_or_else(short)?]))
    };
    Ok(match first >> 4 {
        2 => Packet::ConnAck { code: *body.get(1).ok_or_else(short)? },
        9 => Packet::SubAck { id: u16_at(0)?, codes: body.get(2..).unwrap_or(&[]).to_vec() },
        13 => Packet::PingResp,
        3 => {
            let qos = (first >> 1) & 3;
            let tlen = u16_at(0)? as usize;
            let topic = body.get(2..2 + tlen).ok_or_else(short)?;
            let start = 2 + tlen + if qos > 0 { 2 } else { 0 };
            Packet::Publish { topic: String::from_utf8_lossy(topic).into_owned(), payload: body.get(start..).ok_or_else(short)?.to_vec() }
        }
        t => Packet::Other(t),
    })
}
