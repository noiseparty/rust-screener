//! Settings schema: list columns, Configuration fields and the helpers between inputs and `Config`.

use super::*;

pub struct Column {
    pub key: &'static str,
    pub title: &'static str,
    pub width: u16,
    pub priority: u8,
}

pub const COLUMNS: [Column; 10] = [
    Column { key: "mov", title: "Mov", width: 5, priority: 2 },
    Column { key: "symbol", title: "Symbol", width: 7, priority: 1 },
    Column { key: "price", title: "Price", width: 9, priority: 1 },
    Column { key: "change", title: "% Chg", width: 9, priority: 1 },
    Column { key: "volume", title: "Volume", width: 9, priority: 2 },
    Column { key: "mcap", title: "Mkt Cap", width: 9, priority: 3 },
    Column { key: "float", title: "Float", width: 8, priority: 3 },
    Column { key: "shares", title: "Shares", width: 8, priority: 3 },
    Column { key: "volrate", title: "Vol/m", width: 10, priority: 3 },
    Column { key: "status", title: "Status", width: 7, priority: 1 },
];

#[derive(Clone, Copy, PartialEq)]
pub enum FieldKind {
    Smart,
    Float,
    Int,
    /// One of a fixed list of values, picked with left/right/space instead of typed.
    Choice(&'static [u64]),
    /// On or Off, switched with left/right/space.
    Toggle,
}

/// Name of a Settings field. Everything that needs a field looks it up by name, never by position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldId {
    MinMcap,
    MaxMcap,
    FallbackPoll,
    RankTrack,
    RecomputeMs,
    VolMWindow,
    SetupWindow,
    SetupVolRatio,
    SetupPricePct,
    MinVolAlert,
    MinPriceAlert,
    AlertCooldown,
    MinChange,
    MaxChange,
    MinVolume,
    MaxVolume,
    MinPrice,
    MaxPrice,
    MinFloat,
    MaxFloat,
    MinShares,
    MaxShares,
    TickerTape,
    KeyHints,
    AlertRows,
}

pub struct Field {
    pub id: FieldId,
    pub label: &'static str,
    pub desc: &'static str,
    pub kind: FieldKind,
}

pub const RECOMPUTE_CHOICES: &[u64] = &[500, 1000];
/// `ALERT_ROWS_MIN..=ALERT_ROWS_MAX`.
pub const ALERT_ROW_CHOICES: &[u64] = &{
    let mut rows = [0u64; (crate::config::ALERT_ROWS_MAX - crate::config::ALERT_ROWS_MIN + 1) as usize];
    let mut i = 0;
    while i < rows.len() {
        rows[i] = crate::config::ALERT_ROWS_MIN as u64 + i as u64;
        i += 1;
    }
    rows
};

pub const FIELDS: [Field; 25] = [
    Field {
        id: FieldId::MinMcap,
        label: "Min Market Cap",
        desc: "Minimum market cap to display. K/M/B suffixes allowed (500K, 1M). 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::MaxMcap,
        label: "Max Market Cap",
        desc: "Maximum market cap to display. K/M/B suffixes allowed (1B). 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::FallbackPoll,
        label: "Fallback Poll (s)",
        desc: "How often to poll Webull over REST, in seconds. Used only while the live stream is down.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::RankTrack,
        label: "Rank Track (s)",
        desc: "Window for rank movement (Mov column). 60 = compare to rank 60s ago. Min 5.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::RecomputeMs,
        label: "Recompute (ms)",
        desc: "How often the table re-ranks and re-sorts. Left/right/space to switch between 500 and 1000.",
        kind: FieldKind::Choice(RECOMPUTE_CHOICES),
    },
    Field {
        id: FieldId::VolMWindow,
        label: "Vol/m Window (s)",
        desc: "Window for the Vol/m column (min 30). 180 = average shares per minute over the last 3 minutes.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::SetupWindow,
        label: "SETUP Window (s)",
        desc: "SETUP window. 300 = volume of the last 5 minutes versus the 5 minutes before, price versus 5 minutes ago.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::SetupVolRatio,
        label: "SETUP Volume Ratio",
        desc: "SETUP needs the volume of the last window at least this multiple of the window before it. 2.2 = 2.2x.",
        kind: FieldKind::Float,
    },
    Field {
        id: FieldId::SetupPricePct,
        label: "SETUP Price %",
        desc: "SETUP also needs price up at least this percent over the SETUP window. 10 = +10%.",
        kind: FieldKind::Float,
    },
    Field {
        id: FieldId::MinVolAlert,
        label: "Min Vol Alert",
        desc: "Minimum volume required before SETUP can fire.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::MinPriceAlert,
        label: "Min Price Alert ($)",
        desc: "Minimum price required before SETUP can fire.",
        kind: FieldKind::Float,
    },
    Field {
        id: FieldId::AlertCooldown,
        label: "Alert Cooldown (s)",
        desc: "Seconds before the same alert kind can fire again for one symbol. 120 = 2 minutes.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::MinChange,
        label: "Min % Change",
        desc: "Only show tickers up at least this %. 0 disables.",
        kind: FieldKind::Float,
    },
    Field {
        id: FieldId::MaxChange,
        label: "Max % Change",
        desc: "Only show tickers up at most this %. 0 disables.",
        kind: FieldKind::Float,
    },
    Field {
        id: FieldId::MinVolume,
        label: "Min Volume",
        desc: "Only show tickers with at least this volume. 0 disables.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::MaxVolume,
        label: "Max Volume",
        desc: "Only show tickers with at most this volume. 0 disables.",
        kind: FieldKind::Int,
    },
    Field {
        id: FieldId::MinPrice,
        label: "Min Price",
        desc: "Only show tickers priced at least this. 0 disables.",
        kind: FieldKind::Float,
    },
    Field { id: FieldId::MaxPrice, label: "Max Price", desc: "Only show tickers priced at most this. 0 disables.", kind: FieldKind::Float },
    Field {
        id: FieldId::MinFloat,
        label: "Min Float",
        desc: "Only show tickers with at least this float (shares). K/M/B suffixes allowed (5M). Unknown floats pass. 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::MaxFloat,
        label: "Max Float",
        desc: "Only show tickers with at most this float (shares). K/M/B suffixes allowed (20M). Unknown floats pass. 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::MinShares,
        label: "Min Shares",
        desc: "Only show tickers with at least this many total shares. K/M/B suffixes allowed. Unknown counts pass. 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::MaxShares,
        label: "Max Shares",
        desc: "Only show tickers with at most this many total shares. K/M/B suffixes allowed. Unknown counts pass. 0 disables.",
        kind: FieldKind::Smart,
    },
    Field {
        id: FieldId::TickerTape,
        label: "Ticker Tape",
        desc: "The scrolling ticker tape at the bottom of the dashboard. Off gives its row to the list.",
        kind: FieldKind::Toggle,
    },
    Field {
        id: FieldId::KeyHints,
        label: "Key Hints",
        desc: "The key hints at the bottom of the dashboard. Off gives their row to the list.",
        kind: FieldKind::Toggle,
    },
    Field {
        id: FieldId::AlertRows,
        label: "Alert Rows",
        desc: "Rows of the alerts panel above the list (5-15). Left/right/space to change.",
        kind: FieldKind::Choice(ALERT_ROW_CHOICES),
    },
];

/// Position of a field in `FIELDS` (and in the input list).
pub fn field_index(id: FieldId) -> usize {
    FIELDS.iter().position(|f| f.id == id).expect("every FieldId is listed in FIELDS")
}

/// Text shown in the input box for one field.
pub(super) fn input_text(c: &Config, id: FieldId) -> String {
    match id {
        FieldId::MinMcap => format!("{:.0}", c.min_market_cap),
        FieldId::MaxMcap => format!("{:.0}", c.max_market_cap),
        FieldId::FallbackPoll => c.refresh_interval_seconds.to_string(),
        FieldId::RankTrack => c.rank_tracking_interval_seconds.to_string(),
        FieldId::RecomputeMs => c.recompute_ms.to_string(),
        FieldId::VolMWindow => c.vol_m_window_seconds.to_string(),
        FieldId::SetupWindow => c.setup_window_sec.to_string(),
        FieldId::SetupVolRatio => format!("{:.2}", c.setup_vol_ratio),
        FieldId::SetupPricePct => format!("{:.2}", c.setup_price_pct),
        FieldId::MinVolAlert => c.min_volume_for_alert.to_string(),
        FieldId::MinPriceAlert => format!("{:.2}", c.min_price_for_alert),
        FieldId::AlertCooldown => c.alert_cooldown_sec.to_string(),
        FieldId::MinChange => format!("{:.1}", c.min_change_percent),
        FieldId::MaxChange => format!("{:.1}", c.max_change_percent),
        FieldId::MinVolume => c.min_volume.to_string(),
        FieldId::MaxVolume => c.max_volume.to_string(),
        FieldId::MinPrice => format!("{:.2}", c.min_price),
        FieldId::MaxPrice => format!("{:.2}", c.max_price),
        FieldId::MinFloat => format!("{:.0}", c.min_float),
        FieldId::MaxFloat => format!("{:.0}", c.max_float),
        FieldId::MinShares => format!("{:.0}", c.min_shares),
        FieldId::MaxShares => format!("{:.0}", c.max_shares),
        FieldId::TickerTape => on_off(c.ticker_tape).into(),
        FieldId::KeyHints => on_off(c.key_hints).into(),
        FieldId::AlertRows => c.alert_rows.to_string(),
    }
}

/// Text of a Toggle field.
pub(super) fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

/// Stores one parsed field value in the config (validation happens afterwards, in `Config::validate`).
pub(super) fn apply_field(c: &mut Config, id: FieldId, v: f64) {
    match id {
        FieldId::MinMcap => c.min_market_cap = v,
        FieldId::MaxMcap => c.max_market_cap = v,
        FieldId::FallbackPoll => c.refresh_interval_seconds = v as u64,
        FieldId::RankTrack => c.rank_tracking_interval_seconds = v as u64,
        FieldId::RecomputeMs => c.recompute_ms = v as u64,
        FieldId::VolMWindow => c.vol_m_window_seconds = v as u64,
        FieldId::SetupWindow => c.setup_window_sec = v as u64,
        FieldId::SetupVolRatio => c.setup_vol_ratio = v,
        FieldId::SetupPricePct => c.setup_price_pct = v,
        FieldId::MinVolAlert => c.min_volume_for_alert = v as i64,
        FieldId::MinPriceAlert => c.min_price_for_alert = v,
        FieldId::AlertCooldown => c.alert_cooldown_sec = v as u64,
        FieldId::MinChange => c.min_change_percent = v,
        FieldId::MaxChange => c.max_change_percent = v,
        FieldId::MinVolume => c.min_volume = v as i64,
        FieldId::MaxVolume => c.max_volume = v as i64,
        FieldId::MinPrice => c.min_price = v,
        FieldId::MaxPrice => c.max_price = v,
        FieldId::MinFloat => c.min_float = v,
        FieldId::MaxFloat => c.max_float = v,
        FieldId::MinShares => c.min_shares = v,
        FieldId::MaxShares => c.max_shares = v,
        FieldId::TickerTape => c.ticker_tape = v != 0.0,
        FieldId::KeyHints => c.key_hints = v != 0.0,
        FieldId::AlertRows => c.alert_rows = v as u16,
    }
}

pub(super) fn inputs_from(c: &Config) -> Vec<String> {
    FIELDS.iter().map(|f| input_text(c, f.id)).collect()
}

/// The config after applying every input to `base` and validating. `Err(i)` is the first field
/// whose text does not parse.
pub(super) fn config_from_inputs(base: &Config, inputs: &[String]) -> Result<Config, usize> {
    let mut c = base.clone();
    for (i, f) in FIELDS.iter().enumerate() {
        let v = parse_field(&inputs[i], f.kind).ok_or(i)?;
        apply_field(&mut c, f.id, v);
    }
    c.validate();
    Ok(c)
}

/// Display filters from the (possibly unsaved) inputs; unparseable fields count as 0 (off).
pub fn filters_from_inputs(inputs: &[String]) -> Filters {
    let mut c = Config::default();
    for (i, f) in FIELDS.iter().enumerate() {
        apply_field(&mut c, f.id, parse_field(&inputs[i], f.kind).unwrap_or(0.0));
    }
    Filters::from_config(&c)
}

/// Next (or previous) option after the one in `cur`; an unknown value goes to the first option.
pub(super) fn cycle_choice(cur: &str, opts: &[u64], back: bool) -> String {
    match cur.trim().parse::<u64>() {
        Ok(v) => crate::fmt::cycle(opts, &v, back),
        Err(_) => opts[0],
    }
    .to_string()
}

pub fn parse_field(s: &str, kind: FieldKind) -> Option<f64> {
    let s = s.trim();
    match kind {
        FieldKind::Smart => parse_smart(s),
        FieldKind::Float => s.parse::<f64>().ok().filter(|v| v.is_finite()),
        FieldKind::Int => s.parse::<u64>().ok().map(|v| v as f64),
        FieldKind::Choice(opts) => s.parse::<u64>().ok().filter(|v| opts.contains(v)).map(|v| v as f64),
        FieldKind::Toggle => match s {
            "On" => Some(1.0),
            "Off" => Some(0.0),
            _ => None,
        },
    }
}

/// Parses numbers with optional K/M/B/T suffix and thousands separators.
pub fn parse_smart(s: &str) -> Option<f64> {
    let s = s.trim().to_uppercase().replace([',', ' '], "");
    if s.is_empty() {
        return Some(0.0);
    }
    let (num, mult) = match s.chars().last() {
        Some('K') => (&s[..s.len() - 1], 1e3),
        Some('M') => (&s[..s.len() - 1], 1e6),
        Some('B') => (&s[..s.len() - 1], 1e9),
        Some('T') => (&s[..s.len() - 1], 1e12),
        _ => (s.as_str(), 1.0),
    };
    num.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v * mult)
}
