//! User settings, kept next to the profiles under ~/.config/vpndeck.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// "dark" or "light". The client lives in the tray, so dark is the default.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// "en" or "ru". Follows the desktop language until the user picks one.
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_language() -> String {
    Lang::from_env().code().to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: default_theme(), language: default_language() }
    }
}

impl Settings {
    pub fn load() -> Settings {
        fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }

    pub fn dark(&self) -> bool {
        self.theme != "light"
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.theme = if dark { "dark" } else { "light" }.to_string();
    }

    pub fn lang(&self) -> Lang {
        Lang::from_code(&self.language)
    }

    pub fn set_lang(&mut self, lang: Lang) {
        self.language = lang.code().to_string();
    }
}

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("vpndeck").join("settings.json")
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
    }

    #[test]
    fn reads_and_writes_the_theme() {
        let light: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert!(!light.dark());

        let mut settings = Settings::default();
        settings.set_dark(false);
        assert_eq!(settings.theme, "light");
        assert!(!settings.dark());
        settings.set_dark(true);
        assert_eq!(settings.theme, "dark");
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
}
