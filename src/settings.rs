//! User settings, kept next to the profiles under ~/.config/ether.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// "dark", "light" or "system". The client lives in the tray, so dark is the default.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// "en" or "ru". Follows the desktop language until the user picks one.
    #[serde(default = "default_language")]
    pub language: String,
    /// Start with the desktop session.
    #[serde(default)]
    pub autostart: bool,
    /// Connect to the last used profile as soon as the window opens.
    #[serde(default)]
    pub auto_connect: bool,
    /// Show a desktop notification when a connection starts, drops or fails.
    #[serde(default = "default_notifications")]
    pub notifications: bool,
    /// The window size in logical pixels, so it reopens the way it was left.
    #[serde(default)]
    pub window: Option<[u32; 2]>,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_language() -> String {
    Lang::from_env().code().to_string()
}

fn default_notifications() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: default_theme(),
            language: default_language(),
            autostart: false,
            auto_connect: false,
            notifications: default_notifications(),
            window: None,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let path = path();
        // Older versions left the folder and the file readable by every user.
        if let Some(dir) = path.parent() {
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&path, json)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
    }

    /// Whether the interface should be dark right now. "system" asks the desktop.
    pub fn dark(&self) -> bool {
        match self.theme.as_str() {
            "light" => false,
            "system" => crate::theme::prefers_dark(),
            _ => true,
        }
    }

    /// "dark", "light" or "system".
    pub fn theme_mode(&self) -> &str {
        match self.theme.as_str() {
            "light" => "light",
            "system" => "system",
            _ => "dark",
        }
    }

    pub fn set_theme(&mut self, mode: &str) {
        self.theme = match mode {
            "light" => "light",
            "system" => "system",
            _ => "dark",
        }
        .to_string();
    }

    pub fn lang(&self) -> Lang {
        Lang::from_code(&self.language)
    }

    pub fn set_lang(&mut self, lang: Lang) {
        self.language = lang.code().to_string();
    }

    pub fn window_size(&self) -> Option<(u32, u32)> {
        self.window.map(|[width, height]| (width, height))
    }

    pub fn set_window_size(&mut self, width: u32, height: u32) {
        self.window = Some([width, height]);
    }
}

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("ether").join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_dark() {
        assert!(Settings::default().dark());
        // A settings file written before the theme existed still loads.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.dark());
        assert!(old.notifications);
        assert!(!old.autostart);
        assert!(!old.auto_connect);
        assert_eq!(old.window, None);
    }

    #[test]
    fn reads_and_writes_the_theme() {
        let light: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert!(!light.dark());
        assert_eq!(light.theme_mode(), "light");

        let mut settings = Settings::default();
        settings.set_theme("light");
        assert_eq!(settings.theme, "light");
        assert!(!settings.dark());
        settings.set_theme("dark");
        assert_eq!(settings.theme, "dark");
        // Anything unknown falls back to dark instead of breaking the palette.
        settings.set_theme("purple");
        assert_eq!(settings.theme, "dark");
    }

    #[test]
    fn the_system_theme_is_kept_as_a_mode() {
        let mut settings = Settings::default();
        settings.set_theme("system");
        assert_eq!(settings.theme_mode(), "system");
        // Resolving it asks the desktop, which may answer either way.
        let _ = settings.dark();
    }

    #[test]
    fn reads_and_writes_the_language() {
        let ru: Settings = serde_json::from_str(r#"{"language":"ru"}"#).unwrap();
        assert_eq!(ru.lang(), Lang::Ru);

        let mut settings = Settings::default();
        settings.set_lang(Lang::Ru);
        assert_eq!(settings.language, "ru");
        assert_eq!(settings.lang(), Lang::Ru);
        settings.set_lang(Lang::En);
        assert_eq!(settings.language, "en");
    }

    #[test]
    fn an_unknown_language_falls_back_to_english() {
        let settings: Settings = serde_json::from_str(r#"{"language":"de"}"#).unwrap();
        assert_eq!(settings.lang(), Lang::En);
    }

    #[test]
    fn remembers_the_window_size() {
        let mut settings = Settings::default();
        assert_eq!(settings.window_size(), None);
        settings.set_window_size(520, 800);
        assert_eq!(settings.window_size(), Some((520, 800)));
        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.window_size(), Some((520, 800)));
    }
}
