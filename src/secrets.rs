//! Saved VPN passwords. They live in the desktop keyring (GNOME Keyring, KWallet)
//! through the Secret Service API, never in Ether's own files.

use std::path::Path;

const SERVICE: &str = "ether-ovpn";
/// Every password prompt openvpn can raise for a profile, so they can all be forgotten.
const REALMS: [&str; 2] = ["Auth", "Private Key"];

fn entry(profile: &Path, realm: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{}#{realm}", profile.display())).map_err(|e| e.to_string())
}

pub fn load(profile: &Path, realm: &str) -> Result<Option<String>, String> {
    match entry(profile, realm)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save(profile: &Path, realm: &str, password: &str) -> Result<(), String> {
    entry(profile, realm)?.set_password(password).map_err(|e| e.to_string())
}

pub fn forget_all(profile: &Path) -> Result<(), String> {
    for realm in REALMS {
        match entry(profile, realm)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to the real desktop keyring: cargo test -- --ignored keyring
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        let profile = Path::new("/nonexistent/ether-ovpn-keyring-test.ovpn");
        save(profile, "Auth", "s3cret pass").unwrap();
        assert_eq!(load(profile, "Auth").unwrap().as_deref(), Some("s3cret pass"));
        forget_all(profile).unwrap();
        assert_eq!(load(profile, "Auth").unwrap(), None);
    }
}
