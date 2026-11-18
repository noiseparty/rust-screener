use super::*;

#[test]
fn smart_parse() {
    assert_eq!(parse_smart("500K"), Some(500_000.0));
    assert_eq!(parse_smart("1.5m"), Some(1_500_000.0));
    assert_eq!(parse_smart("2,000"), Some(2000.0));
    assert_eq!(parse_smart(""), Some(0.0));
    assert_eq!(parse_smart("abc"), None);
}

#[test]
fn int_fields_reject_negative_and_decimals() {
    assert_eq!(parse_field("5", FieldKind::Int), Some(5.0));
    assert_eq!(parse_field("-5", FieldKind::Int), None);
    assert_eq!(parse_field("5.5", FieldKind::Int), None);
}

#[test]
fn settings_labels_are_the_planned_ones() {
    let labels: Vec<&str> = FIELDS.iter().map(|f| f.label).collect();
    for want in [
        "Fallback Poll (s)",
        "SETUP Window (s)",
        "SETUP Volume Ratio",
        "SETUP Price %",
        "Alert Cooldown (s)",
        "Recompute (ms)",
        "Vol/m Window (s)",
    ] {
        assert!(labels.contains(&want), "missing {want}");
    }
    for gone in ["Refresh Rate (s)", "Spike Vol Ratio", "Pump Price Pct", "Baseline Window", "Delta Window (s)"] {
        assert!(!labels.contains(&gone), "{gone} still listed");
    }
}

#[test]
fn every_field_id_is_listed_once_and_found_by_name() {
    for (i, f) in FIELDS.iter().enumerate() {
        assert_eq!(field_index(f.id), i);
        assert_eq!(FIELDS.iter().filter(|g| g.id == f.id).count(), 1);
    }
    assert_eq!(FIELDS[field_index(FieldId::RecomputeMs)].label, "Recompute (ms)");
    assert_eq!(FIELDS[field_index(FieldId::FallbackPoll)].label, "Fallback Poll (s)");
    assert_eq!(FIELDS[field_index(FieldId::AlertCooldown)].label, "Alert Cooldown (s)");
    assert_eq!(FIELDS[field_index(FieldId::MaxPrice)].label, "Max Price");
}

#[test]
fn every_field_roundtrips_settings_to_config() {
    let c = Config {
        min_market_cap: 5e8,
        max_market_cap: 2e9,
        refresh_interval_seconds: 7,
        rank_tracking_interval_seconds: 45,
        recompute_ms: 500,
        vol_m_window_seconds: 90,
        setup_window_sec: 600,
        setup_vol_ratio: 3.5,
        setup_price_pct: 7.25,
        min_volume_for_alert: 25_000,
        min_price_for_alert: 0.5,
        alert_cooldown_sec: 45,
        min_change_percent: 2.5,
        max_change_percent: 80.5,
        min_volume: 1_000,
        max_volume: 90_000_000,
        min_price: 0.25,
        max_price: 20.0,
        ticker_tape: false,
        key_hints: false,
        alert_rows: 12,
        ..Config::default()
    };
    let inputs = inputs_from(&c);
    assert_eq!(inputs.len(), FIELDS.len());
    // starting from different values, the inputs bring every field to `c`
    let got = config_from_inputs(&Config::default(), &inputs).unwrap();
    assert_eq!(got, c);
    // fields outside Settings are left alone
    let base = Config { blacklisted_tickers: vec!["ZZZ".into()], stream_enabled: false, ..Config::default() };
    let got = config_from_inputs(&base, &inputs).unwrap();
    assert_eq!(got.blacklisted_tickers, vec!["ZZZ".to_string()]);
    assert!(!got.stream_enabled);
}

#[test]
fn default_config_survives_a_settings_roundtrip() {
    let c = Config::default();
    assert_eq!(config_from_inputs(&c, &inputs_from(&c)).unwrap(), c);
}

#[test]
fn out_of_range_inputs_clamp_like_validate() {
    let mut inputs = inputs_from(&Config::default());
    set(&mut inputs, FieldId::FallbackPoll, "0");
    set(&mut inputs, FieldId::RankTrack, "2");
    set(&mut inputs, FieldId::VolMWindow, "5");
    set(&mut inputs, FieldId::SetupWindow, "0");
    set(&mut inputs, FieldId::AlertCooldown, "0");
    set(&mut inputs, FieldId::SetupVolRatio, "0");
    set(&mut inputs, FieldId::SetupPricePct, "-3");
    let c = config_from_inputs(&Config::default(), &inputs).unwrap();
    assert_eq!(c.refresh_interval_seconds, 1);
    assert_eq!(c.rank_tracking_interval_seconds, 5);
    assert_eq!(c.vol_m_window_seconds, 30);
    assert_eq!(c.setup_window_sec, 300);
    assert_eq!(c.alert_cooldown_sec, 120);
    assert_eq!(c.setup_vol_ratio, 2.2);
    assert_eq!(c.setup_price_pct, 10.0);
}

#[test]
fn unparseable_inputs_are_rejected_with_the_field_index() {
    for (id, bad) in [
        (FieldId::RecomputeMs, "750"),
        (FieldId::RecomputeMs, "abc"),
        (FieldId::RecomputeMs, ""),
        (FieldId::FallbackPoll, "-1"),
        (FieldId::FallbackPoll, "1.5"),
        (FieldId::AlertCooldown, "x"),
        (FieldId::SetupVolRatio, "abc"),
        (FieldId::SetupWindow, ""),
        (FieldId::VolMWindow, "-5"),
    ] {
        let mut inputs = inputs_from(&Config::default());
        set(&mut inputs, id, bad);
        assert_eq!(config_from_inputs(&Config::default(), &inputs), Err(field_index(id)), "{id:?} = {bad:?}");
    }
}

#[test]
fn recompute_is_a_two_way_choice() {
    assert_eq!(parse_field("500", FieldKind::Choice(RECOMPUTE_CHOICES)), Some(500.0));
    assert_eq!(parse_field("1000", FieldKind::Choice(RECOMPUTE_CHOICES)), Some(1000.0));
    assert_eq!(parse_field("250", FieldKind::Choice(RECOMPUTE_CHOICES)), None);
    assert!(matches!(FIELDS[field_index(FieldId::RecomputeMs)].kind, FieldKind::Choice(o) if o == [500, 1000]));
    assert_eq!(cycle_choice("1000", RECOMPUTE_CHOICES, false), "500");
    assert_eq!(cycle_choice("500", RECOMPUTE_CHOICES, false), "1000");
    assert_eq!(cycle_choice("500", RECOMPUTE_CHOICES, true), "1000");
    assert_eq!(cycle_choice("junk", RECOMPUTE_CHOICES, false), "500");
}

#[test]
fn preview_filters_read_fields_by_name() {
    let mut inputs = inputs_from(&Config::default());
    set(&mut inputs, FieldId::MinMcap, "500K");
    set(&mut inputs, FieldId::MaxMcap, "2B");
    set(&mut inputs, FieldId::MinChange, "5");
    set(&mut inputs, FieldId::MaxChange, "90");
    set(&mut inputs, FieldId::MinVolume, "1000");
    set(&mut inputs, FieldId::MaxVolume, "5000000");
    set(&mut inputs, FieldId::MinPrice, "0.5");
    set(&mut inputs, FieldId::MaxPrice, "15");
    // alert and timing fields must not leak into the filters
    set(&mut inputs, FieldId::SetupVolRatio, "9");
    set(&mut inputs, FieldId::AlertCooldown, "77");
    let f = filters_from_inputs(&inputs);
    assert_eq!(f.min_mcap, 500_000.0);
    assert_eq!(f.max_mcap, 2e9);
    assert_eq!((f.min_change_pct, f.max_change_pct), (5.0, 90.0));
    assert_eq!((f.min_volume, f.max_volume), (1000, 5_000_000));
    assert_eq!((f.min_price, f.max_price), (0.5, 15.0));
    // a half-typed value counts as off
    set(&mut inputs, FieldId::MaxPrice, "1.");
    set(&mut inputs, FieldId::MinPrice, "x");
    assert_eq!(filters_from_inputs(&inputs).min_price, 0.0);
}
