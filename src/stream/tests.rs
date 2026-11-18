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
    assert_eq!(pk, vec![Packet::ConnAck { code: 0 }, Packet::SubAck { id: 7, codes: vec![0] }, Packet::PingResp]);
    assert!(buf.is_empty());
}

#[test]
fn parses_publish_qos0_and_qos1() {
    let mut buf = vec![0x30, 7, 0, 2, b't', b'p', 1, 2, 3];
    assert_eq!(parse_packets(&mut buf).unwrap(), vec![Packet::Publish { topic: "tp".into(), payload: vec![1, 2, 3] }]);
    // QoS 1 carries a packet id after the topic.
    let mut buf = vec![0x32, 8, 0, 2, b't', b'p', 0, 9, 4, 5];
    assert_eq!(parse_packets(&mut buf).unwrap(), vec![Packet::Publish { topic: "tp".into(), payload: vec![4, 5] }]);
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
    let base =
        [f_varint(1, 5), f_varint(2, 77), f_varint(3, 950978197), f_str(4, "T"), f_varint(5, 1_763_400_000_123), f_varint(8, 9)].concat();
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
fn raw_decode_keeps_every_field_as_text() {
    let m = decode_raw(&sample(), None).unwrap();
    assert_eq!(m.id, 950978197);
    assert_eq!(m.base.get(&1).map(String::as_str), Some("5"));
    assert_eq!(m.base.get(&4).map(String::as_str), Some("T"));
    assert_eq!(m.base.get(&5).map(String::as_str), Some("1763400000123"));
    assert_eq!(m.base.get(&8).map(String::as_str), Some("9")); // unknown field kept
    assert_eq!(m.sale.get(&7).map(String::as_str), Some("12,500,000")); // not normalised
    assert_eq!(m.stats.get(&52).map(String::as_str), Some("3"));
    assert_eq!(m.modules, vec![(6, 7)]);
}

#[test]
fn raw_decode_id_fallback_and_rejects() {
    let buf = f_bytes(3, &f_str(3, "10"));
    assert_eq!(decode_raw(&buf, Some(7)).unwrap().id, 7);
    assert!(decode_raw(&buf, None).is_none());
    let mut b = sample();
    b.truncate(b.len() - 3);
    assert!(decode_raw(&b, Some(1)).is_none());
    // binary string field is hex-encoded
    let buf = f_bytes(2, &f_bytes(9, &[0xFF, 0x01]));
    assert_eq!(decode_raw(&buf, Some(1)).unwrap().sale.get(&9).map(String::as_str), Some("ff01"));
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
