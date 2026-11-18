//! The Config shell: the sidebar (#1), the selected item's pane (#2) and its preview (#3), and
//! the Configuration, Presets and Theme panes with their previews.

use super::boot::gear;
use super::samples::sample_candles;
use super::setup_panes::{
    alerts_preview, blacklisted_preview, format_preview, health, setup_blacklist, setup_filters, setup_format, setup_sound, storage_view,
};
use super::tables::{cell, count_or_na};
use super::{centered, hint_bar, panel, title};
use crate::app::{App, FieldId, FieldKind, PresetAsk, View, BL_COLS, COLUMNS, FIELDS, SETUP_ITEMS, SETUP_VIEWS};
use crate::config::FILTER_NAMES;
use crate::engine::AlertKind;
use crate::engine::Candle;
use crate::fmt;
use crate::history::Tf;
use crate::scraper::Ticker;
use crate::theme::{self, pal};
use crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

/// Width of the Config sidebar (#1).
pub(super) const SIDEBAR_W: u16 = 26;
/// Settings label column and value column widths in Configuration.
const LABEL_W: usize = 22;
const VALUE_W: usize = 26;
/// Settings pane (#2) widths of the items with a preview (#3) beside it.
const FORM_W: u16 = (LABEL_W + VALUE_W + 10) as u16;
const PRESETS_W: u16 = 36;
const THEME_W: u16 = 34;
const BLACKLIST_W: u16 = BL_COLS as u16 * 12 + 2;
pub(super) const FILTERS_W: u16 = 80;
const FORMAT_W: u16 = 64;

/// Config: the sidebar (#1), the selected item's settings (#2, shown live while the sidebar has
/// focus), its preview (#3) for Configuration, Presets, Theme and Blacklist, and one hint bar.
pub(super) fn setup_shell(f: &mut Frame, app: &mut App) {
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let [side, rest] = Layout::horizontal([Constraint::Length(SIDEBAR_W), Constraint::Min(0)]).areas(body);
    sidebar(f, app, side);
    let item = SETUP_VIEWS[app.setup_idx];
    let active = app.view != View::SetupMenu;
    let pane_w = match item {
        View::SetupConfig => Some(FORM_W),
        View::Presets => Some(PRESETS_W),
        View::SetupTheme => Some(THEME_W),
        View::SetupBlacklist => Some(BLACKLIST_W),
        View::SetupFilters => Some(FILTERS_W),
        View::SetupFormat => Some(FORMAT_W),
        _ => None,
    };
    let (pane, preview) = match pane_w {
        Some(w) if rest.width >= w + 30 => {
            let [a, b] = Layout::horizontal([Constraint::Length(w), Constraint::Min(0)]).areas(rest);
            (a, Some(b))
        }
        _ => (rest, None),
    };
    match item {
        View::SetupConfig => config_form(f, app, pane, active),
        View::Presets => presets_list(f, app, pane),
        View::SetupTheme => theme_list(f, app, pane),
        View::SetupBlacklist => setup_blacklist(f, app, pane),
        View::SetupFilters => setup_filters(f, app, pane),
        View::SetupSound => setup_sound(f, app, pane),
        View::SetupFormat => setup_format(f, app, pane, active),
        View::Storage => storage_view(f, app, pane),
        _ => health(f, app, pane),
    }
    match (item, preview) {
        (View::SetupConfig, Some(p)) => filter_preview(f, &app.snapshot, &app.preview_filters(), p),
        (View::Presets, Some(p)) => preset_preview(f, app, p),
        (View::SetupTheme, Some(p)) => theme_preview(f, app, p),
        (View::SetupBlacklist, Some(p)) => blacklisted_preview(f, app, p),
        (View::SetupFilters, Some(p)) => alerts_preview(f, app, p),
        (View::SetupFormat, Some(p)) => format_preview(f, p),
        _ => {}
    }
    let esc = ("esc", "back", Some(KeyCode::Esc));
    let hints: Vec<(&str, &str, Option<KeyCode>)> = match app.view {
        View::SetupMenu => vec![("↑↓", "item", None), ("enter/→", "open", Some(KeyCode::Enter)), esc],
        View::SetupConfig => vec![
            ("tab/↑↓", "field", None),
            ("←→", "choose", None),
            ("p", "recommended preset", Some(KeyCode::Char('p'))),
            ("enter", "save", Some(KeyCode::Enter)),
            ("esc", "cancel", Some(KeyCode::Esc)),
        ],
        View::Presets if app.preset_input.is_some() => vec![("enter", "save", Some(KeyCode::Enter)), ("esc", "cancel", Some(KeyCode::Esc))],
        View::Presets if app.preset_cursor == app.presets.len() => {
            vec![("↑↓", "move", None), ("enter", "save as…", Some(KeyCode::Enter)), esc]
        }
        View::Presets => {
            vec![("↑↓", "move", None), ("enter", "load", Some(KeyCode::Enter)), ("d", "delete", Some(KeyCode::Char('d'))), esc]
        }
        View::SetupTheme => vec![("↑↓", "preview", None), ("enter", "keep", Some(KeyCode::Enter)), ("esc", "revert", Some(KeyCode::Esc))],
        View::SetupBlacklist => {
            vec![("arrows", "move", None), ("enter", "toggle", Some(KeyCode::Enter)), ("a", "type symbol", Some(KeyCode::Char('a'))), esc]
        }
        View::SetupFilters if app.filter_cursor == FILTER_NAMES.len() => vec![("↑↓", "move", None), ("←→", "rows", None), esc],
        View::SetupFilters => vec![("↑↓", "move", None), ("enter", "toggle", Some(KeyCode::Enter)), esc],
        View::SetupSound if app.sound_cursor == 1 => {
            vec![("↑↓", "move", None), ("←→", "volume", None), ("enter", "test", Some(KeyCode::Enter)), esc]
        }
        View::SetupSound => vec![("↑↓", "move", None), ("enter", "mute on/off", Some(KeyCode::Enter)), esc],
        View::SetupFormat if app.fmt_cursor < fmt::TYPED_ROWS => {
            vec![
                ("↑↓", "move", None),
                ("←→", "choose", None),
                ("type", "custom", None),
                ("enter", "save", Some(KeyCode::Enter)),
                ("esc", "cancel", Some(KeyCode::Esc)),
            ]
        }
        View::SetupFormat => vec![
            ("↑↓", "move", None),
            ("←→", "choose", None),
            ("enter", "save", Some(KeyCode::Enter)),
            ("esc", "cancel", Some(KeyCode::Esc)),
        ],
        View::Storage => vec![
            ("↑↓", "move", None),
            ("enter/→", "open folder", None),
            ("←/bksp", "up", None),
            ("d/del", "delete", Some(KeyCode::Char('d'))),
            ("r", "refresh", Some(KeyCode::Char('r'))),
            esc,
        ],
        _ => vec![("enter", "run again", Some(KeyCode::Enter)), esc],
    };
    hint_bar(f, app, footer, &hints);
}

/// Config sidebar (#1): the items (clickable), the selected one's description, and the gear at
/// the bottom when it fits.
fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.view == View::SetupMenu;
    let block = panel(title("CONFIG"));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut y = inner.y + 1;
    for (i, (name, _)) in SETUP_ITEMS.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        let sel = i == app.setup_idx;
        let st = match (sel, focused) {
            (true, true) => Style::new().fg(pal().on_accent).bg(pal().green).bold(),
            (true, false) => Style::new().fg(pal().accent).bg(pal().select).bold(),
            _ => Style::new().fg(pal().text),
        };
        let r = Rect::new(inner.x, y, inner.width, 1);
        let text = format!(" {} {name:<w$}", if sel { "▶" } else { " " }, w = inner.width.saturating_sub(3) as usize);
        f.render_widget(Paragraph::new(Span::styled(text, st)), r);
        app.hits.setup_items.push((r, i));
        y += 2;
    }
    let desc_h = 4;
    let desc = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), desc_h).intersection(inner);
    let text = Paragraph::new(SETUP_ITEMS[app.setup_idx].1).style(Style::new().fg(pal().dim).italic()).wrap(Wrap { trim: true });
    f.render_widget(text, desc);
    let below = (y + desc_h + 1).min(inner.bottom());
    let t = app.started_at.elapsed().as_secs_f64();
    gear(f, app, Rect::new(inner.x, below, inner.width, inner.bottom() - below), t);
}

/// Configuration (#2): the settings form and the focused field's description; `active` = the
/// form has focus (else no field is marked).
fn config_form(f: &mut Frame, app: &App, form: Rect, active: bool) {
    let section = |name: &str| {
        Line::from(vec![
            Span::styled(format!("  {name} "), Style::new().fg(pal().accent).bold()),
            Span::styled("─".repeat(LABEL_W + VALUE_W - name.len()), Style::new().fg(pal().faint)),
        ])
    };
    let mut lines = vec![Line::raw("")];
    if app.first_run {
        let amber = Style::new().fg(pal().amber).bold();
        lines.push(Line::styled("  First run: set the display filters so the list holds", amber));
        lines.push(Line::styled("  only the tickers you want; charts are loaded for every", amber));
        lines.push(Line::styled("  listed ticker. p loads the recommended preset.", amber));
        lines.push(Line::raw(""));
    }
    lines.push(section("SCREENER & ALERTS"));
    for (i, fld) in FIELDS.iter().enumerate() {
        match fld.id {
            FieldId::MinChange => {
                lines.push(Line::raw(""));
                lines.push(section("DISPLAY FILTERS"));
            }
            FieldId::TickerTape => {
                lines.push(Line::raw(""));
                lines.push(section("LAYOUT"));
            }
            _ => {}
        }
        let focused = active && i == app.focus;
        let label_st = if focused { Style::new().fg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        let mark = if focused { "▶" } else { " " };
        let raw = app.inputs[i].clone();
        // every value starts in the same column; choices put their arrows around it
        let picked = matches!(fld.kind, FieldKind::Choice(_) | FieldKind::Toggle);
        let shown = if focused && !picked { format!("{raw}▏") } else { raw };
        let (open, close) = if picked { ("◂ ", " ▸") } else { ("  ", "  ") };
        let val_st = if focused { Style::new().fg(pal().text).bg(pal().input_bg) } else { Style::new().fg(pal().text) };
        let arrow_st = Style::new().fg(if focused { pal().accent } else { pal().faint });
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} {:<w$}", fld.label, w = LABEL_W), label_st),
            Span::styled(open, arrow_st),
            Span::styled(format!("{shown:<w$}", w = VALUE_W - 4), val_st),
            Span::styled(close, arrow_st),
        ]));
    }
    if let Some(e) = &app.config_err {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("  {e}"), Style::new().fg(pal().red).bold()));
    }

    let [form_top, tip] = Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).areas(form);
    f.render_widget(Paragraph::new(lines).block(panel(title("CONFIGURATION"))), form_top);
    f.render_widget(
        Paragraph::new(FIELDS[app.focus].desc)
            .style(Style::new().fg(pal().tip))
            .wrap(Wrap { trim: true })
            .block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(pal().faint))),
        tip,
    );
}

/// Filter preview (#3): every ticker of `snapshot`, those failing `filters` dimmed.
fn filter_preview(f: &mut Frame, snapshot: &[Ticker], filters: &crate::engine::Filters, preview: Rect) {
    let passing = snapshot.iter().filter(|t| filters.matches(t)).count();
    let rows = snapshot.iter().map(|t| {
        let st = if filters.matches(t) { Style::new().fg(pal().text) } else { Style::new().fg(pal().faint) };
        Row::new(vec![
            Cell::from(t.symbol.clone()),
            Cell::from(Line::from(fmt::price(t.price)).alignment(Alignment::Right)),
            Cell::from(Line::from(format!("{:+.1}%", t.change_ratio * 100.0)).alignment(Alignment::Right)),
            Cell::from(Line::from(fmt::count(t.volume as f64)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.market_value)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.float)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.total_shares)).alignment(Alignment::Right)),
        ])
        .style(st)
    });
    let header = Row::new(["Symbol", "Price", "% Chg", "Volume", "Mkt Cap", "Float", "Shares"].map(|h| {
        let l = Line::from(Span::styled(h, Style::new().fg(pal().dim).bold()));
        Cell::from(if h == "Symbol" { l } else { l.alignment(Alignment::Right) })
    }));
    let widths = [8, 10, 9, 10, 10, 9, 9].map(Constraint::Length);
    let preview = Rect { width: preview.width.min(90), ..preview };
    let ttl = Line::from(vec![
        Span::styled(" Filter preview ", Style::new().fg(pal().accent).bold()),
        Span::styled(format!("{passing}/{} pass ", snapshot.len()), Style::new().fg(pal().dim)),
    ]);
    f.render_widget(Table::new(rows, widths).header(header).column_spacing(1).block(panel(ttl)), preview);
    if snapshot.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No data yet", Style::new().fg(pal().faint))), centered(preview, 11, 1));
    }
}

/// Presets (#2): the built-in preset and the saved ones, the "Save current config as" row, the
/// name prompt, a question or message.
fn presets_list(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![Line::raw("")];
    for (i, name) in app.presets.iter().enumerate() {
        let sel = i == app.preset_cursor;
        let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        lines.push(Line::from(vec![
            Span::raw(if sel { " ▶ " } else { "   " }),
            Span::styled(format!(" {name} "), st),
            Span::styled(if crate::presets::is_builtin(name) { " built in" } else { "" }, Style::new().fg(pal().dim).italic()),
        ]));
    }
    let sel = app.preset_cursor == app.presets.len();
    let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().accent) };
    lines.push(Line::from(vec![Span::raw(if sel { " ▶ " } else { "   " }), Span::styled(" + Save current config as… ", st)]));
    lines.push(Line::raw(""));
    if let Some(buf) = &app.preset_input {
        lines.push(Line::styled(" Save the current config as:", Style::new().fg(pal().accent)));
        lines.push(Line::from(vec![Span::raw(" "), Span::styled(format!("{buf}▏"), Style::new().fg(pal().text).bg(pal().input_bg))]));
    }
    let amber = Style::new().fg(pal().amber).bold();
    match &app.preset_ask {
        Some(PresetAsk::Overwrite(n)) => lines.push(Line::styled(format!(" Replace preset {n}? y = yes, any other key = no"), amber)),
        Some(PresetAsk::Delete(n)) => lines.push(Line::styled(format!(" Delete preset {n}? y = yes, any other key = no"), amber)),
        None => {}
    }
    if let Some(m) = &app.preset_msg {
        lines.push(Line::styled(format!(" {m}"), Style::new().fg(pal().dim)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title("PRESETS"))), area);
}

/// Presets preview (#3): the selected preset's (or, on the save row, the config in use's)
/// Configuration values and its filter preview.
fn preset_preview(f: &mut Frame, app: &App, area: Rect) {
    let name = app.presets.get(app.preset_cursor).cloned().unwrap_or_else(|| "Current config".to_string());
    let Some(inputs) = &app.preset_inputs else {
        let msg = Line::styled(" this preset could not be read", Style::new().fg(pal().faint));
        f.render_widget(Paragraph::new(msg).block(panel(title(&name))), area);
        return;
    };
    let [vals, prev] = Layout::vertical([Constraint::Length(FIELDS.len() as u16 + 3), Constraint::Min(3)]).areas(area);
    let lines: Vec<Line> = FIELDS
        .iter()
        .zip(inputs)
        .map(|(fld, v)| {
            Line::from(vec![
                Span::styled(format!("  {:<w$}", fld.label, w = LABEL_W), Style::new().fg(pal().dim)),
                Span::styled(v.clone(), Style::new().fg(pal().text)),
            ])
        })
        .collect();
    let mut lines = lines;
    if let Some(s) = &app.preset_summary {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<w$}", "Time & Format", w = LABEL_W), Style::new().fg(pal().dim)),
            Span::styled(s.clone(), Style::new().fg(pal().text)),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(panel(title(&name))), Rect { width: vals.width.min(90), ..vals });
    filter_preview(f, &app.snapshot, &crate::app::filters_from_inputs(inputs), prev);
}

/// Theme (#2): every theme, the highlighted one previewed on the whole screen, the saved one marked.
fn theme_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = theme::names()
        .iter()
        .map(|(id, name)| {
            let saved = if *id == app.cfg.theme { " in use" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {name} "), Style::new().fg(pal().text)),
                Span::styled(saved, Style::new().fg(pal().dim).italic()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(panel(title("THEME")))
        .highlight_symbol(" ▶")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
        .highlight_style(Style::new().fg(pal().on_accent).bg(pal().accent).bold());
    let mut state = ListState::default().with_selected(Some(app.theme_cursor));
    f.render_stateful_widget(list, area, &mut state);
}

/// Theme preview (#3): a small dashboard in the previewed theme: a few gainers rows and the
/// charted (or first) ticker's chart; sample rows and candles while there is no data.
fn theme_preview(f: &mut Frame, app: &App, area: Rect) {
    let area = Rect { width: area.width.min(90), ..area };
    let mut rows: Vec<Ticker> = if app.tickers.is_empty() { app.snapshot.clone() } else { app.tickers.clone() };
    if rows.is_empty() {
        let mk = |s: &str, price, chg, volume, status: Option<AlertKind>| Ticker {
            symbol: s.into(),
            price,
            change_ratio: chg,
            volume,
            status,
            ..Default::default()
        };
        rows = vec![
            mk("SMPL", 3.42, 0.524, 18_400_000, Some(AlertKind::Setup)),
            mk("DEMO", 1.18, 0.317, 6_200_000, Some(AlertKind::New)),
            mk("TEST", 7.05, 0.211, 2_900_000, Some(AlertKind::Pump)),
            mk("ABCD", 0.62, -0.048, 950_000, None),
        ];
    }
    rows.truncate(6);
    let [list, chart] = Layout::vertical([Constraint::Length(rows.len() as u16 + 3), Constraint::Min(0)]).areas(area);
    let keys = ["mov", "symbol", "price", "change", "volume", "status"];
    let widths: Vec<Constraint> =
        keys.iter().map(|k| Constraint::Length(COLUMNS.iter().find(|c| c.key == *k).map_or(8, |c| c.width))).collect();
    let header = Row::new(keys.map(|k| {
        let ttl = COLUMNS.iter().find(|c| c.key == k).map_or("", |c| c.title);
        Cell::from(Span::styled(ttl, Style::new().fg(pal().dim).bold()))
    }));
    let body = rows.iter().enumerate().map(|(i, t)| {
        let st = if i == 0 {
            Style::new().bg(pal().select).fg(pal().text)
        } else if i % 2 == 1 {
            Style::new().bg(pal().zebra)
        } else {
            Style::new()
        };
        Row::new(keys.map(|k| cell(t, k))).style(st)
    });
    let name = theme::names().get(app.theme_cursor).map_or(String::new(), |(_, n)| n.to_string());
    f.render_widget(Table::new(body, widths).header(header).column_spacing(1).block(panel(title(&format!("Preview: {name}")))), list);

    let sym = app.charted_symbol().unwrap_or(&rows[0].symbol).to_string();
    let tf = app.timeframe;
    let mut candles: Vec<Candle> = match tf {
        Tf::M1 => app.engine.candles(&sym).map(|c| c.iter().copied().collect()).unwrap_or_default(),
        _ => app.bars.get(&(sym.clone(), tf)).cloned().unwrap_or_default(),
    };
    if candles.is_empty() {
        candles = sample_candles(tf.secs());
    }
    let pct = rows.iter().find(|t| t.symbol == sym).map_or(0.0, |t| t.change_ratio * 100.0);
    let block = panel(title("CHART"));
    let inner = block.inner(chart).inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(block, chart);
    let day_axis = matches!(tf, Tf::H4 | Tf::D1);
    let now = chrono::Utc::now().timestamp();
    let spec = crate::chart::Spec {
        tf,
        day_axis,
        empty: "",
        pan: 0,
        zoom: 1.0,
        shift: 0.0,
        levels: &[],
        note: "",
        scale: crate::chart::DEFAULT_SCALE,
        pixel: false,
        now,
    };
    crate::chart::draw(f.buffer_mut(), inner, &sym, &candles, pct, &spec);
}
