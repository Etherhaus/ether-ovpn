//! Imported profiles, kept under ~/.config/vpndeck.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Why a file was left out of a bulk import.
#[derive(Clone, Debug, PartialEq)]
pub enum SkipReason {
    /// The same profile is already in the list.
    Duplicate,
    /// Not an .ovpn or .conf file, or not a text file at all.
    NotProfile,
    /// Reading or copying the file failed.
    Failed(String),
}

/// What a bulk import did, so the UI can report it.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<usize>,
    pub skipped: Vec<(String, SkipReason)>,
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
    fn import_one(&mut self, src: &Path) -> io::Result<usize> {
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
        Ok(self.profiles.len() - 1)
    }

    /// Imports every file that isn't already in the list, saving once at the end.
    ///
    /// Files are matched by content, so re-importing the same profile under a
    /// different name is reported as a duplicate instead of piling up copies.
    pub fn import_many(&mut self, paths: &[PathBuf]) -> ImportReport {
        let mut report = ImportReport::default();
        let mut known: Vec<[u8; 32]> = self
            .profiles
            .iter()
            .filter_map(|p| fs::read(&p.file).ok().map(|bytes| digest(&bytes)))
            .collect();

        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
            let bytes = match read_profile(path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                    report.skipped.push((name, SkipReason::NotProfile));
                    continue;
                }
                Err(e) => {
                    report.skipped.push((name, SkipReason::Failed(e.to_string())));
                    continue;
                }
            };
            let digest = digest(&bytes);
            if known.contains(&digest) {
                report.skipped.push((name, SkipReason::Duplicate));
                continue;
            }
            match self.import_one(path) {
                Ok(index) => {
                    known.push(digest);
                    report.imported.push(index);
                }
                Err(e) => report.skipped.push((name, SkipReason::Failed(e.to_string()))),
            }
        }

        if !report.imported.is_empty() {
            let _ = self.save();
        }
        report
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

/// Reads a profile, rejecting anything that isn't an .ovpn/.conf text file.
fn read_profile(path: &Path) -> io::Result<Vec<u8>> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    if extension != "ovpn" && extension != "conf" {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an .ovpn file"));
    }
    let bytes = fs::read(path)?;
    // A profile is a text file; a NUL byte means the user picked something else.
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an .ovpn file"));
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in a throwaway directory, so tests never touch ~/.config.
    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("vpndeck-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Store { dir, profiles: Vec::new() }
    }

    fn profile_file(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn imports_several_files_at_once() {
        let mut store = store("many");
        let src = store.dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let a = profile_file(&src, "work.ovpn", "client\nremote vpn.example.com 1194\n");
        let b = profile_file(&src, "home.ovpn", "client\nremote home.example.com 1194\n");

        let report = store.import_many(&[a, b]);

        assert_eq!(report.imported, vec![0, 1]);
        assert!(report.skipped.is_empty());
        assert_eq!(store.profiles.len(), 2);
        assert_eq!(store.profiles[0].name, "work");
        assert_eq!(store.profiles[1].name, "home");
        // The copies are private to the user.
        let mode = fs::metadata(&store.profiles[0].file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn skips_a_profile_that_is_already_there() {
        let mut store = store("dupe");
        let src = store.dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let a = profile_file(&src, "work.ovpn", "client\nremote vpn.example.com 1194\n");
        let copy = profile_file(&src, "work-copy.ovpn", "client\nremote vpn.example.com 1194\n");

        store.import_many(&[a]);
        let report = store.import_many(&[copy]);

        assert!(report.imported.is_empty());
        assert_eq!(report.skipped, vec![("work-copy.ovpn".to_string(), SkipReason::Duplicate)]);
        assert_eq!(store.profiles.len(), 1);
    }

    #[test]
    fn skips_files_that_are_not_profiles() {
        let mut store = store("junk");
        let src = store.dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let text = profile_file(&src, "notes.txt", "client\nremote vpn.example.com 1194\n");
        let binary = profile_file(&src, "image.ovpn", "client\0remote\n");

        let report = store.import_many(&[text, binary]);

        assert!(report.imported.is_empty());
        assert_eq!(report.skipped.len(), 2);
        assert!(report.skipped.iter().all(|(_, reason)| *reason == SkipReason::NotProfile));
        assert!(store.profiles.is_empty());
    }

    #[test]
    fn gives_repeated_names_a_suffix() {
        let mut store = store("names");
        // Same file name in two folders, which is how two servers usually arrive.
        let first = store.dir.join("src").join("one");
        let second = store.dir.join("src").join("two");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let a = profile_file(&first, "work.ovpn", "client\nremote one.example.com 1194\n");
        let b = profile_file(&second, "work.ovpn", "client\nremote two.example.com 1194\n");

        let report = store.import_many(&[a, b]);

        assert_eq!(report.imported, vec![0, 1]);
        assert_eq!(store.profiles[0].name, "work");
        assert_eq!(store.profiles[1].name, "work (2)");
    }
}
