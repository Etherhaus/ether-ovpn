//! Imported profiles, kept under ~/.config/vpndeck.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Our private copy of the .ovpn file.
    pub file: PathBuf,
    /// Where the profile was imported from, so relative cert/key paths still resolve.
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    #[serde(default)]
    pub username: String,
    /// Whether a password for this profile is stored in the keyring.
    #[serde(default)]
    pub saved_password: bool,
}

pub struct Store {
    dir: PathBuf,
    pub profiles: Vec<Profile>,
}

impl Store {
    pub fn load() -> Store {
        let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("vpndeck");
        let profiles = fs::read_to_string(dir.join("profiles.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Store { dir, profiles }
    }

    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.dir.join("profiles.json"), serde_json::to_string_pretty(&self.profiles)?)
    }

    /// Copies `src` into the store and returns the new profile's index.
    pub fn import(&mut self, src: &Path) -> io::Result<usize> {
        let base = src.file_stem().and_then(|s| s.to_str()).unwrap_or("profile").to_string();
        let mut name = base.clone();
        for n in 2.. {
            if !self.profiles.iter().any(|p| p.name == name) {
                break;
            }
            name = format!("{base} ({n})");
        }

        // Profiles usually embed private keys, so keep them readable by the user only.
        let dir = self.dir.join("profiles");
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let stem: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let mut file = dir.join(format!("{stem}.ovpn"));
        for n in 2.. {
            if !file.exists() {
                break;
            }
            file = dir.join(format!("{stem}-{n}.ovpn"));
        }
        fs::copy(src, &file)?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;

        // Files picked through the document portal live in a FUSE mount that
        // root (openvpn) can't enter, so those fall back to our own copy's folder.
        let workdir = fs::canonicalize(src)
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .filter(|dir| !(dir.starts_with("/run/user") && dir.components().any(|c| c.as_os_str() == "doc")));
        self.profiles.push(Profile { name, file, workdir, username: String::new(), saved_password: false });
        self.save()?;
        Ok(self.profiles.len() - 1)
    }

    pub fn remove(&mut self, index: usize) -> io::Result<Profile> {
        let profile = self.profiles.remove(index);
        if profile.file.starts_with(&self.dir) {
            let _ = fs::remove_file(&profile.file);
        }
        self.save()?;
        Ok(profile)
    }
}
