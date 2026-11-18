use super::*;
use crate::engine::ranking::compare;
use crate::scraper::Mov;
use crate::session::Session;
use std::cmp::Ordering;

#[test]
fn compare_ties_break_by_symbol() {
    let a = tk("AAA", 1.0, 10, 0.0, 1.0);
    let b = tk("BBB", 1.0, 10, 0.0, 1.0);
    for key in ["mov", "price", "change", "volume", "status"] {
        assert_eq!(compare(&a, &b, key), Ordering::Less);
    }
}

fn rank_tk(sym: &str, id: u64, chg: f64) -> Ticker {
    Ticker { ticker_id: id, prev_close: 1.0, ..tk(sym, 1.0 + chg, 1000, chg, 0.0) }
}

fn movs(out: &Output) -> Vec<(String, Mov)> {
    let mut v: Vec<_> = out.active.iter().map(|t| (t.symbol.clone(), t.mov)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn mv(s: &str, m: Mov) -> (String, Mov) {
    (s.to_string(), m)
}

#[test]
fn mov_is_rank_change_over_the_interval() {
    let mut e = engine();
    let t0 = Instant::now();
    let board = |a: f64, b: f64, c: f64| vec![tk("A", 1.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0), tk("C", 1.0, 1, c, 0.0)];
    // no history yet: blank
    let out = e.process(board(0.5, 0.3, 0.1), "change", true, t0);
    assert!(out.active.iter().all(|t| t.mov == Mov::None));
    // 59 s in: the baseline is not a full interval old
    let out = e.process(board(0.2, 0.4, 0.1), "change", true, t0 + Duration::from_secs(59));
    assert!(out.active.iter().all(|t| t.mov == Mov::None));
    // 60 s in: ranks A1 B2 C3 became B1 A2 C3
    let out = e.process(board(0.2, 0.4, 0.1), "change", true, t0 + Duration::from_secs(60));
    assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(1)), mv("C", Mov::Moved(0))]);
    assert_eq!(out.active[0].symbol, "B");
}

#[test]
fn mov_baseline_is_the_board_one_interval_ago() {
    let mut e = engine();
    let t0 = Instant::now();
    let at = |s| t0 + Duration::from_secs(s);
    let board = |a: f64, b: f64| vec![tk("A", 1.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0)];
    e.process(board(0.5, 0.3), "change", true, at(0)); // A B
    e.process(board(0.3, 0.5), "change", true, at(30)); // B A
    e.process(board(0.5, 0.3), "change", true, at(55)); // A B again
                                                        // baseline for t=70 is the board at t=10: A B. Nothing moved, whatever happened in between.
    let out = e.process(board(0.5, 0.3), "change", true, at(70));
    assert_eq!(movs(&out), [mv("A", Mov::Moved(0)), mv("B", Mov::Moved(0))]);
    // baseline for t=95 is the board at t=35: B A
    let out = e.process(board(0.5, 0.3), "change", true, at(95));
    assert_eq!(movs(&out), [mv("A", Mov::Moved(1)), mv("B", Mov::Moved(-1))]);
}

#[test]
fn mov_marks_new_entrants_and_ignores_the_blacklist() {
    let mut e = engine();
    let t0 = Instant::now();
    e.process(vec![tk("A", 1.0, 1, 0.3, 0.0), tk("B", 1.0, 1, 0.2, 0.0)], "change", true, t0);
    // D enters on top; blacklisted X sits above A and B but is not ranked at all
    e.blacklist.insert("X".into());
    let out = e.process(
        vec![tk("D", 1.0, 1, 0.9, 0.0), tk("X", 1.0, 1, 0.8, 0.0), tk("A", 1.0, 1, 0.3, 0.0), tk("B", 1.0, 1, 0.2, 0.0)],
        "change",
        true,
        t0 + Duration::from_secs(60),
    );
    // A and B each drop one place behind D; X has no Mov
    assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(-1)), mv("D", Mov::New)]);
    assert_eq!(out.blacklisted[0].mov, Mov::None);
    // X was not on the board at the start of the interval, so un-blacklisted it counts as a new entrant
    e.blacklist.clear();
    e.recompute(t0 + Duration::from_secs(61));
    let out = e.reapply("change", true, t0 + Duration::from_secs(61));
    assert!(out.active.iter().any(|t| t.symbol == "X" && t.mov == Mov::New));
}

#[test]
fn mov_ignores_sort_and_display_filters() {
    let mut e = engine();
    let t0 = Instant::now();
    let board = |a: f64, b: f64, c: f64| vec![tk("A", 5.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0), tk("C", 3.0, 1, c, 0.0)];
    e.process(board(0.5, 0.3, 0.1), "change", true, t0);
    let t1 = t0 + Duration::from_secs(60);
    let by_change = e.process(board(0.1, 0.3, 0.5), "change", true, t1);
    let by_price = e.reapply("price", false, t1);
    assert_eq!(movs(&by_change), movs(&by_price));
    assert_eq!(movs(&by_change), [mv("A", Mov::Moved(-2)), mv("B", Mov::Moved(0)), mv("C", Mov::Moved(2))]);
    // a filter that hides B and C leaves the full-board ranks untouched
    let cfg = Config { min_price: 4.0, max_market_cap: 0.0, ..Config::default() };
    e.apply_config(&cfg);
    let out = e.reapply("change", true, t1);
    assert_eq!(out.active.len(), 1);
    assert_eq!(out.active[0].mov, Mov::Moved(-2));
}

#[test]
fn sorting_by_mov_puts_blank_and_new_rows_last() {
    let mut e = engine();
    let t0 = Instant::now();
    e.process(vec![tk("A", 1.0, 1, 0.5, 0.0), tk("B", 1.0, 1, 0.3, 0.0), tk("C", 1.0, 1, 0.1, 0.0)], "change", true, t0);
    let t1 = t0 + Duration::from_secs(60);
    // new board B C N A: A lost 3, B and C gained 1, N is new
    let out = e.process(
        vec![tk("A", 1.0, 1, 0.1, 0.0), tk("B", 1.0, 1, 0.5, 0.0), tk("C", 1.0, 1, 0.3, 0.0), tk("N", 1.0, 1, 0.2, 0.0)],
        "mov",
        true,
        t1,
    );
    let order = |o: &Output| o.active.iter().map(|t| t.symbol.as_str()).collect::<String>();
    assert_eq!(order(&out), "CBAN");
    assert_eq!(order(&e.reapply("mov", false, t1)), "ABCN");
    // before any history every row is blank: symbol order
    let mut fresh = engine();
    let out = fresh.process(vec![tk("B", 1.0, 1, 0.3, 0.0), tk("A", 1.0, 1, 0.5, 0.0)], "mov", true, t0);
    assert_eq!(order(&out), "AB");
}

#[test]
fn recompute_tick_reranks_from_stream_quotes() {
    let mut e = engine();
    let t0 = Instant::now();
    e.process(vec![rank_tk("A", 1, 0.5), rank_tk("B", 2, 0.3)], "change", true, t0);
    // stream data arrives; Mov is not recomputed until the tick
    assert!(e.apply_quotes(&[upd(1, 1000, 1, 1.1, 2000), upd(2, 1000, 1, 1.9, 2000)], Session::Reg, t0));
    let t1 = t0 + Duration::from_secs(60);
    let out = e.reapply("change", true, t1);
    assert!(out.active.iter().all(|t| t.mov == Mov::None));
    assert_eq!(out.active[0].symbol, "B");
    e.recompute(t1);
    let out = e.reapply("change", true, t1);
    assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(1))]);
}
