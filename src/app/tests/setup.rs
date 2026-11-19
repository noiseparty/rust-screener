use super::*;

#[test]
fn theme_previews_while_browsing_esc_drops_it_and_enter_saves_it() {
    let (mut app, _rx) = test_app(Config::default());
    assert_eq!(app.palette, theme::NOIR);
    let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupTheme).unwrap();
    assert_eq!(SETUP_ITEMS[item].0, "Theme");
    assert!(FIELDS.iter().all(|f| f.label != "Theme"));
    app.select_setup(item);
    app.view = View::SetupTheme;
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    assert_eq!(app.theme_cursor, 0);
    app.on_key(key(KeyCode::Down));
    let (second, third) = (theme::names()[1].0, theme::names()[2].0);
    assert_eq!(app.palette, theme::palette(second));
    // up stops at the first
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Up));
    assert_eq!((app.theme_cursor, app.palette), (0, theme::NOIR));
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Esc));
    assert!(app.view == View::SetupMenu);
    assert_eq!((app.palette, app.theme_cursor), (theme::NOIR, 0));
    assert_eq!(app.cfg.theme, theme::DEFAULT);
    // leaving the item drops a preview too
    app.view = View::SetupTheme;
    app.on_key(key(KeyCode::Down));
    app.select_setup(item + 1);
    assert_eq!(app.palette, theme::NOIR);
    // pick and save; the cursor starts on the saved theme next time
    app.select_setup(item);
    app.view = View::SetupTheme;
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.cfg.theme, second);
    assert_eq!(app.palette, theme::palette(second));
    app.select_setup(item);
    assert_eq!(app.theme_cursor, 1);
    app.view = View::SetupTheme;
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.palette, theme::palette(second));
    assert_ne!(app.palette, theme::palette(third));
}

#[test]
fn p_loads_the_recommended_preset_into_the_inputs_and_enter_saves_it() {
    let (mut app, _rx) = test_app(Config::default());
    app.inputs = inputs_from(&app.cfg);
    app.view = View::SetupConfig;
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    // previewed only
    assert_eq!(app.cfg.min_change_percent, 0.0);
    assert_eq!(app.inputs[field_index(FieldId::MinChange)], "15.0");
    // presets apply the Configuration fields only, never the theme
    assert_eq!(app.palette, theme::NOIR);
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let want = Config::recommended(&Config::default());
    assert_eq!((app.cfg.min_change_percent, app.cfg.max_price, app.cfg.max_market_cap), (15.0, 25.0, 800_000_000.0));
    assert_eq!((app.cfg.vol_m_window_seconds, app.cfg.refresh_interval_seconds), (want.vol_m_window_seconds, 1));
    assert_eq!(app.cfg.theme, theme::DEFAULT);
    assert_eq!(app.palette, theme::NOIR);
}

#[test]
fn config_sidebar_shows_items_live_and_focuses_their_pane() {
    let (mut app, _rx) = test_app(Config::default());
    app.view = View::Dashboard;
    app.running = true;
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    app.on_key(key(KeyCode::Char('o')));
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.setup_idx, 0);
    // moving the sidebar keeps it focused; the pane follows
    app.on_key(key(KeyCode::Down));
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.setup_idx, 1);
    assert_eq!(app.presets, vec![presets::BUILTIN.to_string()]);
    for _ in 0..10 {
        app.on_key(key(KeyCode::Down));
    }
    assert_eq!(app.setup_idx, SETUP_ITEMS.len() - 1);
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::Health);
    app.on_key(key(KeyCode::Esc));
    assert!(app.view == View::SetupMenu);
    // Configuration: Esc drops unsaved inputs and goes back to the sidebar
    app.select_setup(0);
    app.on_key(key(KeyCode::Right));
    assert!(app.view == View::SetupConfig);
    app.inputs[field_index(FieldId::MinPrice)] = "7".into();
    app.on_key(key(KeyCode::Esc));
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.inputs, inputs_from(&app.cfg));
    // a click on another item selects it, on the shown one opens it
    let filters = SETUP_VIEWS.iter().position(|&v| v == View::SetupFilters).unwrap();
    app.hits.setup_items = vec![(Rect::new(0, 3, 24, 1), 0), (Rect::new(0, 5, 24, 1), filters)];
    let click = |y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 2, row: y, modifiers: KeyModifiers::NONE };
    app.on_mouse(click(5));
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.setup_idx, filters);
    app.on_mouse(click(5));
    assert!(app.view == View::SetupFilters);
    app.on_key(key(KeyCode::Esc));
    app.on_key(key(KeyCode::Esc));
    assert!(app.view == View::Dashboard);
}

#[test]
fn presets_save_load_overwrite_and_delete() {
    let home = std::path::PathBuf::from("target").join("test-app-presets");
    let _ = std::fs::remove_dir_all(&home);
    let (mut app, _rx) = test_app(Config { min_price: 2.0, ..Config::default() });
    app.home = Some(home.clone());
    app.running = true;
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let typed = |app: &mut App, s: &str| s.chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
    app.open_setup();
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::Presets);
    // the built-in preset is listed first and cannot be deleted
    app.on_key(key(KeyCode::Char('d')));
    assert!(app.preset_ask.is_none());
    // the last row ("Save current config as") previews the config in use; Enter asks a name
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.preset_cursor, 1);
    assert_eq!(app.preset_inputs, Some(inputs_from(&app.cfg)));
    app.on_key(key(KeyCode::Char('n'))); // no key of its own any more
    assert!(app.preset_input.is_none());
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.preset_input.as_deref(), Some(""));
    typed(&mut app, "Mine/1");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.presets, vec![presets::BUILTIN.to_string(), "Mine1".into()]);
    assert_eq!(app.preset_cursor, 1);
    assert_eq!(app.preset_inputs.as_ref().unwrap()[field_index(FieldId::MinPrice)], "2.00");
    // the same name asks before replacing; any key but y keeps the old one
    app.cfg.min_price = 3.0;
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "Mine1");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.preset_ask, Some(PresetAsk::Overwrite("Mine1".into())));
    app.on_key(key(KeyCode::Char('x')));
    assert_eq!(presets::load(&presets::dir(&home), "Mine1", &app.cfg).unwrap().min_price, 2.0);
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "Mine1");
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Char('y')));
    assert_eq!(presets::load(&presets::dir(&home), "Mine1", &app.cfg).unwrap().min_price, 3.0);
    // Enter loads it into the Configuration form, previewed; Enter there saves it
    app.cfg.min_price = 0.0;
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::SetupConfig);
    assert_eq!(app.setup_idx, 0);
    assert_eq!(app.inputs[field_index(FieldId::MinPrice)], "3.00");
    assert_eq!(app.cfg.min_price, 0.0);
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.cfg.min_price, 3.0);
    // delete after y (the cursor stayed on Mine1); the cursor then stays on a preset
    app.select_setup(1);
    app.view = View::Presets;
    assert_eq!(app.preset_cursor, 1);
    app.on_key(key(KeyCode::Char('d')));
    assert_eq!(app.preset_ask, Some(PresetAsk::Delete("Mine1".into())));
    app.on_key(key(KeyCode::Char('y')));
    assert_eq!(app.presets, vec![presets::BUILTIN.to_string()]);
    assert_eq!(app.preset_cursor, 0);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn recompute_field_toggles_with_keys_and_ignores_typing() {
    use crossterm::event::KeyEvent;
    let (mut app, _rx) = test_app(Config::default());
    app.view = View::SetupConfig;
    app.focus = field_index(FieldId::RecomputeMs);
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    app.on_key(key(KeyCode::Char('7')));
    app.on_key(key(KeyCode::Backspace));
    assert_eq!(app.inputs[app.focus], "1000");
    app.on_key(key(KeyCode::Right));
    assert_eq!(app.inputs[app.focus], "500");
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(app.inputs[app.focus], "1000");
    app.on_key(key(KeyCode::Left));
    assert_eq!(app.inputs[app.focus], "500");
}

#[test]
fn layout_fields_switch_with_keys_and_save() {
    use crossterm::event::KeyEvent;
    let (mut app, _rx) = test_app(Config::default());
    app.view = View::SetupConfig;
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    app.focus = field_index(FieldId::TickerTape);
    assert_eq!(app.inputs[app.focus], "On");
    app.on_key(key(KeyCode::Char('x')));
    app.on_key(key(KeyCode::Right));
    assert_eq!(app.inputs[app.focus], "Off");
    app.focus = field_index(FieldId::AlertRows);
    app.on_key(key(KeyCode::Left));
    assert_eq!(app.inputs[app.focus], "15");
    app.on_key(key(KeyCode::Enter));
    assert_eq!((app.cfg.ticker_tape, app.cfg.key_hints, app.cfg.alert_rows), (false, true, 15));
    assert_eq!(parse_field("on", FieldKind::Toggle), None);
}

#[test]
fn alert_filters_pane_sets_dashboard_rows() {
    use crossterm::event::KeyEvent;
    let (mut app, _rx) = test_app(Config::default());
    app.view = View::SetupFilters;
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    for _ in 0..5 {
        app.on_key(key(KeyCode::Down));
    }
    assert_eq!(app.filter_cursor, FILTER_NAMES.len());
    app.on_key(key(KeyCode::Left));
    assert_eq!(app.cfg.alert_rows, 15);
    app.on_key(key(KeyCode::Right));
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(app.cfg.alert_rows, 6);
    // on a filter row left still goes back to the sidebar
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Left));
    assert!(app.view == View::SetupMenu);
}

#[test]
fn preview_filters_follow_the_app_inputs() {
    let (mut app, _rx) = test_app(Config::default());
    let i = field_index(FieldId::MinPrice);
    app.inputs[i] = "3".into();
    assert_eq!(app.preview_filters().min_price, 3.0);
}

#[test]
fn saving_settings_applies_the_config_without_a_fetch() {
    let (mut app, rx) = test_app(Config { max_market_cap: 0.0, ..Config::default() });
    app.inputs = inputs_from(&app.cfg);
    set(&mut app.inputs, FieldId::AlertCooldown, "30");
    set(&mut app.inputs, FieldId::RecomputeMs, "500");
    set(&mut app.inputs, FieldId::FallbackPoll, "9");
    set(&mut app.inputs, FieldId::SetupWindow, "120");
    app.view = View::SetupConfig;
    app.commit_config();
    assert!(app.view == View::SetupMenu);
    assert_eq!(app.cfg.alert_cooldown_sec, 30);
    assert_eq!(app.cfg.recompute_ms, 500);
    assert_eq!(app.cfg.refresh_interval_seconds, 9);
    assert_eq!(app.cfg.setup_window_sec, 120);
    assert!(rx.try_recv().is_err(), "saving settings must not trigger a fetch");

    // an invalid field keeps the screen open, focuses the field and changes nothing
    app.view = View::SetupConfig;
    set(&mut app.inputs, FieldId::RecomputeMs, "750");
    app.commit_config();
    assert!(app.view == View::SetupConfig);
    assert_eq!(app.focus, field_index(FieldId::RecomputeMs));
    assert_eq!(app.config_err.as_deref(), Some("Invalid Recompute (ms)"));
    assert_eq!(app.cfg.recompute_ms, 500);
}

#[test]
fn sound_pane_toggles_mute_and_steps_volume() {
    use crossterm::event::KeyEvent;
    let (mut app, _rx) = test_app(Config::default());
    let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupSound).unwrap();
    assert_eq!(SETUP_ITEMS[item].0, "Sound");
    app.view = View::SetupSound;
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    app.on_key(key(KeyCode::Enter));
    assert!(app.cfg.sound_muted);
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Right));
    assert_eq!(app.cfg.sound_volume, 75);
    for _ in 0..10 {
        app.on_key(key(KeyCode::Right));
    }
    assert_eq!(app.cfg.sound_volume, 100);
    for _ in 0..25 {
        app.on_key(key(KeyCode::Left));
    }
    assert_eq!(app.cfg.sound_volume, 0);
    // Enter on the Volume row plays a test and changes nothing
    app.on_key(key(KeyCode::Enter));
    assert!(app.cfg.sound_muted && app.view == View::SetupSound);
    // on the Mute row left goes back to the sidebar
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Left));
    assert!(app.view == View::SetupMenu);
}

#[test]
fn format_pane_previews_validates_saves_and_reverts() {
    let (mut app, _rx) = test_app(Config::default());
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupFormat).unwrap();
    assert_eq!(SETUP_ITEMS[item].0, "Time & Format");
    app.select_setup(item);
    app.view = View::SetupFormat;
    // a typed zone: shown at once, saved only when it is a real one
    app.on_key(key(KeyCode::Delete));
    "Asia/Nowhere".chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
    assert_eq!(app.formats().timezone, "Asia/Nowhere");
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::SetupFormat && app.cfg.formats.timezone == "Local");
    for _ in 0.."Nowhere".len() {
        app.on_key(key(KeyCode::Backspace));
    }
    "Tokyo".chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
    // the clock row: space switches it, Enter saves everything
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::SetupMenu);
    assert_eq!((app.cfg.formats.timezone.as_str(), app.cfg.formats.clock_12h), ("Asia/Tokyo", true));
    // Esc drops unsaved changes; away from the pane the saved formats are drawn
    app.view = View::SetupFormat;
    app.on_key(key(KeyCode::Right));
    assert_ne!(app.formats().clock_12h, app.cfg.formats.clock_12h);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.fmt_draft, app.cfg.formats);
    app.view = View::Dashboard;
    assert_eq!(app.formats(), &app.cfg.formats);
}

#[test]
fn presets_carry_their_formats_into_configuration() {
    let home = std::path::PathBuf::from("target").join("test-app-preset-fmt");
    let _ = std::fs::remove_dir_all(&home);
    let saved = Config { formats: Formats { date: "YYYY-MM-DD".into(), ..Formats::default() }, ..Config::default() };
    presets::save(&presets::dir(&home), "Iso", &saved).unwrap();
    let (mut app, _rx) = test_app(Config::default());
    app.home = Some(home.clone());
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    app.select_setup(1);
    app.view = View::Presets;
    app.on_key(key(KeyCode::Down));
    assert!(app.preset_summary.as_deref().is_some_and(|s| s.contains('-')));
    // loaded: the form previews them, Esc drops them, loading again and Enter saves them
    app.on_key(key(KeyCode::Enter));
    assert!(app.view == View::SetupConfig);
    assert_eq!(app.formats().date, "YYYY-MM-DD");
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.formats().date, "DD.MM.YY");
    // the Presets cursor stays on Iso
    app.select_setup(1);
    app.view = View::Presets;
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.cfg.formats.date, "YYYY-MM-DD");
    assert!(app.preset_formats.is_none());
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn layouts_pane_builds_a_custom_tab_enter_saves_esc_drops() {
    use crate::layouts::{Panel, Template};
    let (mut app, _rx) = test_app(Config::default());
    let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupLayouts).unwrap();
    assert_eq!(SETUP_ITEMS[item].0, "Layouts");
    app.select_setup(item);
    app.view = View::SetupLayouts;
    let key = |app: &mut App, c| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    // Show: on; Name: typed (j and k are letters here)
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Down);
    for c in "Mk j".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    // Template: two columns -> 2 rows -> left | right split
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.layout_draft().template, Template::LeftRight2);
    assert_eq!(app.layout_draft().slots, vec![Panel::Chart, Panel::List, Panel::Empty]);
    // Slot A: Chart -> Bubbles; Slot C: Empty -> Alerts (List is in Slot B, which keeps it)
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.layout_draft().slots, vec![Panel::Bubbles, Panel::List, Panel::Alerts]);
    // nothing saved before Enter
    assert!(app.cfg.custom_layouts.is_empty());
    key(&mut app, KeyCode::Enter);
    assert!(app.view == View::SetupMenu);
    let saved = &app.cfg.custom_layouts[0];
    assert_eq!((saved.on, saved.name.as_str(), saved.template), (true, "Mk", Template::LeftRight2));
    assert_eq!(app.cfg.tabs().last(), Some(&Layout::Custom1));
    assert_eq!(app.cfg.layout_label(Layout::Custom1), "Mk");
    // Esc drops a change
    app.select_setup(item);
    app.view = View::SetupLayouts;
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    assert!(!app.layout_draft().on);
    key(&mut app, KeyCode::Esc);
    assert!(app.view == View::SetupMenu && app.cfg.custom_layouts[0].on && app.layout_draft().on);
    // switched off while shown: the dashboard goes back to Full
    app.cfg.layout = Layout::Custom1;
    app.view = View::SetupLayouts;
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!((app.cfg.layout, app.cfg.tabs().len()), (Layout::Full, 5));
}
