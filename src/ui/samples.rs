//! Made-up data for the Config previews while there is no real data.

use crate::engine::{AlertKind, Candle};

/// Made-up candles (a steady climb with pullbacks) for previews without data, `secs` apart.
pub(super) fn sample_candles(secs: i64) -> Vec<Candle> {
    let now = chrono::Utc::now().timestamp() / secs * secs;
    let mut close = 2.0;
    (0..80)
        .map(|i| {
            let open = close;
            close = (open + ((i as f64) * 0.7).sin() * 0.06 + 0.012).max(0.5);
            let (hi, lo) = (open.max(close) + 0.03, open.min(close) - 0.03);
            let volume = 20_000 + ((i * 7919) % 50) as i64 * 2_000;
            Candle { start: now - (79 - i) * secs, open, high: hi, low: lo, close, volume }
        })
        .collect()
}

/// `n` made-up alerts cycling through the kinds, newest first.
pub(super) fn sample_alerts(n: usize) -> Vec<crate::engine::Alert> {
    let syms = ["SMPL", "DEMO", "TEST", "ABCD", "WXYZ"];
    let kinds = [AlertKind::Setup, AlertKind::New, AlertKind::Pump, AlertKind::Spike];
    (0..n)
        .map(|i| {
            let kind = kinds[i % kinds.len()];
            let now = chrono::Utc::now().timestamp();
            crate::engine::Alert {
                at: std::time::Instant::now(),
                epoch: now - i as i64 * 37,
                symbol: syms[i % syms.len()].to_string(),
                kind,
                prev_vol: 400_000,
                curr_vol: 400_000 + (i as i64 % 5 + 2) * 300_000,
                prev_price: 2.0,
                curr_price: 2.0 + (i % 4) as f64 * 0.12,
                change_ratio: 0.18 + i as f64 * 0.013,
            }
        })
        .collect()
}
