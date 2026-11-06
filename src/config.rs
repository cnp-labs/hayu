use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: Option<i64>,
    pub appearance: Appearance,
    pub bar: Bar,
    pub clock: Clock,
    pub launcher: Launcher,
    pub tray: Tray,
    pub notifications: Notifications,
    pub control_center: ControlCenter,
    pub media: Media,
    pub osd: Osd,
    pub wallpaper: Wallpaper,
    pub dock: Dock,
    pub clipboard: Clipboard,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub mode: String,
    pub accent: String,
    pub animations: bool,
    pub reduce_motion: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Bar {
    pub enabled: bool,
    pub position: String,
    pub height: i64,
    pub outputs: Vec<String>,
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Launcher {
    pub enabled: bool,
    pub favorites: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Tray {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub dnd: bool,
    pub history_limit: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ControlCenter {
    pub audio: bool,
    pub brightness: bool,
    pub network: bool,
    pub battery: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Media {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Osd {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    pub enabled: bool,
    pub directories: Vec<String>,
    pub mode: String,
    pub outputs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Dock {
    pub enabled: bool,
    pub pinned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Clipboard {
    pub enabled: bool,
    pub max_entries: i64,
    pub max_item_bytes: i64,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: "dark".into(),
            accent: "#8AB4F8".into(),
            animations: false,
            reduce_motion: false,
        }
    }
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            enabled: true,
            position: "top".into(),
            height: 32,
            outputs: vec!["*".into()],
            left: vec![
                "launcher".into(),
                "workspaces".into(),
                "active_window".into(),
            ],
            center: vec!["clock".into()],
            right: vec![
                "media".into(),
                "audio".into(),
                "network".into(),
                "battery".into(),
                "tray".into(),
                "notifications".into(),
                "control_center".into(),
            ],
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            format: "%H:%M".into(),
        }
    }
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            enabled: true,
            favorites: Vec::new(),
        }
    }
}

impl Default for Tray {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            dnd: false,
            history_limit: 100,
        }
    }
}

impl Default for ControlCenter {
    fn default() -> Self {
        Self {
            audio: true,
            brightness: true,
            network: true,
            battery: true,
        }
    }
}

impl Default for Media {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Osd {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            enabled: true,
            directories: Vec::new(),
            mode: "fill".into(),
            outputs: BTreeMap::new(),
        }
    }
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            enabled: true,
            pinned: Vec::new(),
        }
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: 100,
            max_item_bytes: 65536,
        }
    }
}

const ROOT_KEYS: &[&str] = &[
    "schema_version",
    "appearance",
    "bar",
    "clock",
    "launcher",
    "tray",
    "notifications",
    "control_center",
    "media",
    "osd",
    "wallpaper",
    "dock",
    "clipboard",
];

const TABLES: &[(&str, &[&str])] = &[
    (
        "appearance",
        &["mode", "accent", "animations", "reduce_motion"],
    ),
    (
        "bar",
        &[
            "enabled", "position", "height", "outputs", "left", "center", "right",
        ],
    ),
    ("clock", &["format"]),
    ("launcher", &["enabled", "favorites"]),
    ("tray", &["enabled"]),
    ("notifications", &["enabled", "dnd", "history_limit"]),
    (
        "control_center",
        &["audio", "brightness", "network", "battery"],
    ),
    ("media", &["enabled"]),
    ("osd", &["enabled"]),
    ("wallpaper", &["enabled", "directories", "mode", "outputs"]),
    ("dock", &["enabled", "pinned"]),
    ("clipboard", &["enabled", "max_entries", "max_item_bytes"]),
];

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("hayu").join("config.toml")
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text =
        std::fs::read_to_string(path).map_err(|e| ConfigError(format!("cannot read file: {e}")))?;
    parse(&text)
}

/// Keys that a reload cannot apply to the running shell. `hayu reload` reports
/// them so a user does not assume a full restart-free change.
pub fn restart_required(old: &Config, new: &Config) -> Vec<&'static str> {
    let mut keys = Vec::new();
    let checks = [
        (
            old.appearance.animations != new.appearance.animations,
            "appearance.animations",
        ),
        (
            old.appearance.reduce_motion != new.appearance.reduce_motion,
            "appearance.reduce_motion",
        ),
        (old.bar.left != new.bar.left, "bar.left"),
        (old.bar.center != new.bar.center, "bar.center"),
        (old.bar.right != new.bar.right, "bar.right"),
        (
            old.launcher.favorites != new.launcher.favorites,
            "launcher.favorites",
        ),
        (old.tray != new.tray, "tray"),
        (old.notifications != new.notifications, "notifications"),
        (old.control_center != new.control_center, "control_center"),
        (old.media != new.media, "media"),
        (old.osd != new.osd, "osd"),
        (old.wallpaper != new.wallpaper, "wallpaper"),
        (old.dock != new.dock, "dock"),
        (old.clipboard != new.clipboard, "clipboard"),
    ];
    for (changed, name) in checks {
        if changed {
            keys.push(name);
        }
    }
    keys
}

pub fn parse(text: &str) -> Result<Config, ConfigError> {
    if text.len() > 1024 * 1024 {
        return Err(ConfigError("config is larger than the 1 MiB limit".into()));
    }
    let value: toml::Value =
        toml::from_str(text).map_err(|e| ConfigError(format!("invalid TOML: {e}")))?;
    check_unknown(&value)?;
    let cfg: Config =
        toml::from_str(text).map_err(|e| ConfigError(format!("invalid config: {e}")))?;
    validate(&cfg)?;
    Ok(cfg)
}

fn unknown(path: &str, key: &str) -> ConfigError {
    let full = if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    };
    ConfigError(format!("{full}: unknown key"))
}

fn check_unknown(value: &toml::Value) -> Result<(), ConfigError> {
    let root = value
        .as_table()
        .ok_or_else(|| ConfigError("top level must be a table".into()))?;
    for (key, val) in root {
        if !ROOT_KEYS.contains(&key.as_str()) {
            return Err(unknown("", key));
        }
        if key == "schema_version" {
            continue;
        }
        let Some(table) = val.as_table() else {
            continue;
        };
        let Some((name, allowed)) = TABLES.iter().find(|(name, _)| name == key) else {
            continue;
        };
        for child in table.keys() {
            if !allowed.contains(&child.as_str()) {
                return Err(unknown(name, child));
            }
        }
    }
    Ok(())
}

fn validate(cfg: &Config) -> Result<(), ConfigError> {
    match cfg.schema_version {
        Some(1) => {}
        Some(v) if v > 1 => {
            return Err(ConfigError(format!(
                "schema_version: {v} is newer than supported version 1"
            )));
        }
        Some(v) => return Err(ConfigError(format!("schema_version: expected 1, got {v}"))),
        None => return Err(ConfigError("schema_version: missing (expected 1)".into())),
    }
    enum_value("appearance.mode", &cfg.appearance.mode, &["dark", "light"])?;
    if !is_hex_color(&cfg.appearance.accent) {
        return Err(ConfigError(format!(
            "appearance.accent: expected #RGB or #RRGGBB, got {:?}",
            cfg.appearance.accent
        )));
    }
    enum_value("bar.position", &cfg.bar.position, &["top", "bottom"])?;
    in_range("bar.height", cfg.bar.height, 24, 96)?;
    in_range(
        "notifications.history_limit",
        cfg.notifications.history_limit,
        1,
        1000,
    )?;
    in_range("clipboard.max_entries", cfg.clipboard.max_entries, 1, 1000)?;
    in_range(
        "clipboard.max_item_bytes",
        cfg.clipboard.max_item_bytes,
        1024,
        1_048_576,
    )?;
    enum_value("wallpaper.mode", &cfg.wallpaper.mode, &["fit", "fill"])?;
    Ok(())
}

fn enum_value(key: &str, value: &str, allowed: &[&str]) -> Result<(), ConfigError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ConfigError(format!(
            "{key}: expected one of {}, got {value:?}",
            allowed
                .iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

fn in_range(key: &str, value: i64, min: i64, max: i64) -> Result<(), ConfigError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError(format!(
            "{key}: expected an integer in {min}..={max}, got {value}"
        )))
    }
}

pub fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn parse_color(value: &str) -> [u8; 3] {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let expand = |s: &str| u8::from_str_radix(s, 16).unwrap_or(0);
    if hex.len() == 3 {
        let d = |i: usize| {
            let c = hex.as_bytes()[i] as char;
            expand(&format!("{c}{c}"))
        };
        [d(0), d(1), d(2)]
    } else if hex.len() == 6 {
        [expand(&hex[0..2]), expand(&hex[2..4]), expand(&hex[4..6])]
    } else {
        [0x8A, 0xB4, 0xF8]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_example() {
        let cfg = parse(include_str!("../examples/config.toml")).expect("example must be valid");
        assert_eq!(cfg.bar.height, 32);
        assert_eq!(cfg.clock.format, "%H:%M");
        assert!(!cfg.appearance.animations);
    }

    #[test]
    fn unknown_key_has_path() {
        let err = parse("schema_version = 1\n[bar]\nhite = 40\n").unwrap_err();
        assert!(err.to_string().contains("bar.hite"), "{err}");
    }

    #[test]
    fn bad_range_has_path() {
        let err = parse("schema_version = 1\n[bar]\nheight = 20\n").unwrap_err();
        assert!(err.to_string().contains("bar.height"), "{err}");
    }

    #[test]
    fn restart_required_lists_only_unapplied_keys() {
        let base = Config::default();
        let applied = parse(
            "schema_version = 1\n[appearance]\nmode = \"light\"\naccent = \"#ffffff\"\n[bar]\nheight = 40\nposition = \"bottom\"\n[clock]\nformat = \"%H\"\n[launcher]\nenabled = false\n",
        )
        .unwrap();
        assert!(restart_required(&base, &applied).is_empty());
        let restart = parse(
            "schema_version = 1\n[bar]\nenabled = false\noutputs = [\"DP-1\"]\nleft = [\"clock\"]\n[launcher]\nfavorites = [\"foot.desktop\"]\n[tray]\nenabled = false\n",
        )
        .unwrap();
        assert_eq!(
            restart_required(&base, &restart),
            ["bar.left", "launcher.favorites", "tray"],
            "bar.enabled and bar.outputs are applied by sync_bars"
        );
        assert_eq!(
            restart_required(&base, &base),
            Vec::<&'static str>::new(),
            "an identical config needs no restart"
        );
    }
    #[test]
    fn future_schema_rejected() {
        let err = parse("schema_version = 2\n").unwrap_err();
        assert!(err.to_string().contains("newer"), "{err}");
    }

    #[test]
    fn bad_enum_and_color_rejected() {
        assert!(parse("schema_version = 1\n[appearance]\nmode = \"dim\"\n").is_err());
        assert!(parse("schema_version = 1\n[appearance]\naccent = \"blue\"\n").is_err());
        assert!(is_hex_color("#8AB4F8"));
        assert!(is_hex_color("#abc"));
        assert!(!is_hex_color("#8AB4F"));
    }
}
