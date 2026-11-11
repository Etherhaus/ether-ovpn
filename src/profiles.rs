//! Imported profiles, kept under ~/.config/ether.

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
    /// `host:port` from the profile's `remote` line, shown before connecting.
    #[serde(default)]
    pub remote: String,
    /// Unix seconds of the last connection, so the tray can offer it again.
    #[serde(default)]
    pub last_used: u64,
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

/// Why a profile couldn't be renamed.
#[derive(Clone, Debug, PartialEq)]
pub enum RenameError {
    /// The name was blank.
    Empty,
    /// Another profile already has that name.
    Taken,
    /// Writing the list failed.
    Save(String),
}

/// What a bulk import did, so the UI can report it.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<usize>,
    pub skipped: Vec<(String, SkipReason)>,
    /// Directives that make OpenVPN run a program, which it does as root.
    pub scripts: Vec<String>,
}

pub struct Store {
    dir: PathBuf,
    pub profiles: Vec<Profile>,
}

impl Store {
    pub fn load() -> Store {
        let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("ether");
        let mut profiles: Vec<Profile> = fs::read_to_string(dir.join("profiles.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // Profiles imported before the server was recorded get it filled in now.
        let mut filled = false;
        for profile in &mut profiles {
            if profile.remote.is_empty()
                && let Ok(text) = fs::read_to_string(&profile.file)
            {
                profile.remote = remote_of(&text);
                filled = true;
            }
        }
        let store = Store { dir, profiles };
        // Older versions left the folder and the list readable by every user.
        let _ = fs::set_permissions(&store.dir, fs::Permissions::from_mode(0o700));
        let _ = fs::set_permissions(store.dir.join("profiles.json"), fs::Permissions::from_mode(0o600));
        if filled {
            let _ = store.save();
        }
        store
    }

    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700))?;
        let path = self.dir.join("profiles.json");
        fs::write(&path, serde_json::to_string_pretty(&self.profiles)?)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
    }

    /// Copies `bytes` into the store under `base` and returns the new profile's index.
    fn store_copy(&mut self, base: &str, bytes: &[u8], workdir: Option<PathBuf>) -> io::Result<usize> {
        let base = if base.trim().is_empty() { "profile" } else { base.trim() };
        let mut name = base.to_string();
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
        fs::write(&file, bytes)?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;

        let remote = remote_of(&String::from_utf8_lossy(bytes));
        self.profiles.push(Profile {
            name,
            file,
            workdir,
            username: String::new(),
            saved_password: false,
            remote,
            last_used: 0,
        });
        Ok(self.profiles.len() - 1)
    }

    /// Copies `src` into the store and returns the new profile's index.
    fn import_one(&mut self, src: &Path, bytes: &[u8]) -> io::Result<usize> {
        let base = src.file_stem().and_then(|s| s.to_str()).unwrap_or("profile").to_string();
        // Files picked through the document portal live in a FUSE mount that
        // root (openvpn) can't enter, so those fall back to our own copy's folder.
        let workdir = fs::canonicalize(src)
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .filter(|dir| !(dir.starts_with("/run/user") && dir.components().any(|c| c.as_os_str() == "doc")));
        self.store_copy(&base, bytes, workdir)
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
            match self.import_one(path, &bytes) {
                Ok(index) => {
                    known.push(digest);
                    report.imported.push(index);
                    collect_scripts(&mut report.scripts, &bytes);
                }
                Err(e) => report.skipped.push((name, SkipReason::Failed(e.to_string()))),
            }
        }

        if !report.imported.is_empty() {
            let _ = self.save();
        }
        report
    }

    /// Imports a profile pasted as text, e.g. from a support ticket.
    pub fn import_text(&mut self, name: &str, text: &str) -> Result<usize, SkipReason> {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.contains(&0) {
            return Err(SkipReason::NotProfile);
        }
        let hash = digest(bytes);
        if self.profiles.iter().filter_map(|p| fs::read(&p.file).ok()).any(|b| digest(&b) == hash) {
            return Err(SkipReason::Duplicate);
        }
        let index = self.store_copy(name, bytes, None).map_err(|e| SkipReason::Failed(e.to_string()))?;
        let _ = self.save();
        Ok(index)
    }

    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), RenameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RenameError::Empty);
        }
        if self.profiles.iter().enumerate().any(|(i, p)| i != index && p.name == name) {
            return Err(RenameError::Taken);
        }
        let Some(profile) = self.profiles.get_mut(index) else { return Err(RenameError::Empty) };
        profile.name = name.to_string();
        self.save().map_err(|e| RenameError::Save(e.to_string()))
    }

    /// Records that this profile was just connected to.
    ///
    /// The stamp is kept strictly increasing, so two connections within the same
    /// second still order correctly.
    pub fn touch(&mut self, index: usize) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let latest = self.profiles.iter().map(|p| p.last_used).max().unwrap_or(0);
        let stamp = now.max(latest + 1);
        if let Some(profile) = self.profiles.get_mut(index) {
            profile.last_used = stamp;
            let _ = self.save();
        }
    }

    /// The profile connected to most recently, for the tray's "connect to last".
    pub fn last_used(&self) -> Option<usize> {
        self.profiles
            .iter()
            .enumerate()
            .filter(|(_, p)| p.last_used > 0)
            .max_by_key(|(_, p)| p.last_used)
            .map(|(index, _)| index)
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

/// The `remote host port` line of a profile, as `host:port`.
pub fn remote_of(text: &str) -> String {
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("remote") {
            continue;
        }
        let Some(host) = words.next() else { continue };
        return match words.next() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
    }
    String::new()
}

/// Directives that make OpenVPN run a program. It runs as root, so a profile
/// carrying one of these can do anything on the machine.
const SCRIPT_DIRECTIVES: [&str; 12] = [
    "up",
    "down",
    "plugin",
    "script-security",
    "ipchange",
    "route-up",
    "route-pre-down",
    "learn-address",
    "client-connect",
    "client-disconnect",
    "tls-verify",
    "auth-user-pass-verify",
];

/// The script-running directives a profile contains, without repeats.
pub fn risky_directives(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some(word) = line.split_whitespace().next() else { continue };
        if SCRIPT_DIRECTIVES.contains(&word) && !found.iter().any(|f| f == word) {
            found.push(word.to_string());
        }
    }
    found
}

fn collect_scripts(into: &mut Vec<String>, bytes: &[u8]) {
    for directive in risky_directives(&String::from_utf8_lossy(bytes)) {
        if !into.contains(&directive) {
            into.push(directive);
        }
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in a throwaway directory, so tests never touch ~/.config.
    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ether-ovpn-test-{name}-{}", std::process::id()));
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
        assert_eq!(store.profiles[0].remote, "vpn.example.com:1194");
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

    #[test]
    fn imports_pasted_text_and_skips_a_repeat() {
        let mut store = store("paste");
        let text = "client\ndev tun\nremote pasted.example.com 1194\n";

        let index = store.import_text("from ticket", text).unwrap();
        assert_eq!(index, 0);
        assert_eq!(store.profiles[0].name, "from ticket");
        assert_eq!(store.profiles[0].remote, "pasted.example.com:1194");
        assert_eq!(store.import_text("again", text), Err(SkipReason::Duplicate));
        assert_eq!(store.import_text("empty", ""), Err(SkipReason::NotProfile));
    }

    #[test]
    fn renames_a_profile() {
        let mut store = store("rename");
        store.import_text("first", "client\nremote a.example.com 1194\n").unwrap();
        store.import_text("second", "client\nremote b.example.com 1194\n").unwrap();

        store.rename(0, "  Office  ").unwrap();
        assert_eq!(store.profiles[0].name, "Office");
        assert_eq!(store.rename(0, "second"), Err(RenameError::Taken));
        assert_eq!(store.rename(0, "   "), Err(RenameError::Empty));
        // Renaming to its own name is allowed.
        assert!(store.rename(0, "Office").is_ok());
    }

    #[test]
    fn remembers_the_last_used_profile() {
        let mut store = store("last");
        store.import_text("a", "client\nremote a.example.com 1194\n").unwrap();
        store.import_text("b", "client\nremote b.example.com 1194\n").unwrap();
        assert_eq!(store.last_used(), None);

        store.touch(1);
        assert_eq!(store.last_used(), Some(1));
        store.touch(0);
        assert_eq!(store.last_used(), Some(0));
    }

    #[test]
    fn reads_the_remote_line() {
        assert_eq!(remote_of("client\nremote vpn.example.com 1194\n"), "vpn.example.com:1194");
        assert_eq!(remote_of("remote vpn.example.com\n"), "vpn.example.com");
        assert_eq!(remote_of("client\n"), "");
        assert_eq!(remote_of(""), "");
    }

    #[test]
    fn finds_script_directives() {
        let profile = "client\nup /tmp/evil.sh\ndown /tmp/evil.sh\n# plugin ignored\nplugin /tmp/x.so\n";
        assert_eq!(risky_directives(profile), vec!["up", "down", "plugin"]);
        assert!(risky_directives("client\nremote a 1194\n").is_empty());
        // A commented-out directive is not one.
        assert!(risky_directives("# up /tmp/evil.sh\n").is_empty());
    }

    #[test]
    fn reports_scripts_found_during_a_bulk_import() {
        let mut store = store("scripts");
        let src = store.dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let a = profile_file(&src, "a.ovpn", "client\nup /tmp/evil.sh\n");
        let b = profile_file(&src, "b.ovpn", "client\nup /tmp/other.sh\nplugin /tmp/x.so\n");

        let report = store.import_many(&[a, b]);

        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.scripts, vec!["up", "plugin"]);
    }
}
