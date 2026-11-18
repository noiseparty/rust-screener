//! Screens and the fixed menus of the boot screen and the Config sidebar.

/// The app's name: window, title bar, boot screen.
pub const APP_NAME: &str = crate::paths::APP_DIR_NAME;
pub const BOOT_ITEMS: [&str; 2] = ["Start MarketGUI", "Config"];
/// Config sidebar items and their descriptions.
pub const SETUP_ITEMS: [(&str, &str); 9] = [
    ("Configuration", "Thresholds, filters and windows, with live filter preview"),
    ("Presets", "Saved configurations: load one into Configuration, save the current one"),
    ("Theme", "Colour theme: browse with a live dashboard preview, Enter keeps it"),
    ("Time & Format", "Time zone, date and clock, currency and number formats, with a live preview"),
    ("Blacklist", "Hide tickers from ranking (still tracked, listed separately)"),
    ("Alert Filters", "Which alert types appear under ALL and in the f cycle"),
    ("Sound", "Alert sound: mute it, set its volume, play a test"),
    ("Storage", "Ticker data on disk: browse it and delete what you no longer need"),
    ("Health Check", "Connection checks for the network and the stock data source"),
];
/// The settings pane (#2) of each sidebar item, in `SETUP_ITEMS` order.
pub const SETUP_VIEWS: [View; 9] = [
    View::SetupConfig,
    View::Presets,
    View::SetupTheme,
    View::SetupFormat,
    View::SetupBlacklist,
    View::SetupFilters,
    View::SetupSound,
    View::Storage,
    View::Health,
];
/// Volume step of the Sound pane's slider (left/right).
pub const VOLUME_STEP: u8 = 5;
pub const BL_COLS: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Boot,
    Health,
    Dashboard,
    /// Config with the sidebar focused; the other setup views are its focused settings pane.
    SetupMenu,
    SetupConfig,
    SetupBlacklist,
    SetupFilters,
    Columns,
    Chart,
    Storage,
    Presets,
    /// Config -> Theme: the theme list, previewed while browsing.
    SetupTheme,
    /// Config -> Sound: mute and volume of the alert sound.
    SetupSound,
    /// Config -> Time & Format: time zone, date, clock and number formats.
    SetupFormat,
}

/// A Presets action waiting for `y`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresetAsk {
    Overwrite(String),
    Delete(String),
}

/// Trace mode's debug overlay: available while tracing (`on`), toggled on the dashboard;
/// `scroll` = log lines hidden below the shown ones (scrolled back).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DebugView {
    pub on: bool,
    pub open: bool,
    pub scroll: usize,
}
