//! Checks a profile before connecting, so most failures show up before the
//! admin password prompt rather than after it.
//!
//! Everything here runs without root: it reads the profile, looks for the files
//! it points at, resolves the server and, for a TCP profile, opens a socket.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::profiles::Profile;

/// What a check looked at, so the interface can name it in the user's language.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    File,
    Remote,
    Files,
    Dns,
    Port,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Passed,
    Failed(String),
    /// The check doesn't apply, e.g. a TCP probe on a UDP profile.
    Skipped(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub kind: Kind,
    pub outcome: Outcome,
}

const TIMEOUT: Duration = Duration::from_secs(5);

/// Directives that name a file. An inline `<ca>` block carries the certificate
/// itself and has no file name, so those lines are skipped.
const FILE_DIRECTIVES: [&str; 8] = [
    "ca",
    "cert",
    "key",
    "tls-auth",
    "tls-crypt",
    "pkcs12",
    "auth-user-pass",
    "crl-verify",
];

pub fn run(profile: &Profile) -> Vec<Check> {
    let text = match std::fs::read_to_string(&profile.file) {
        Ok(text) => text,
        Err(e) => return vec![Check { kind: Kind::File, outcome: Outcome::Failed(e.to_string()) }],
    };
    let mut checks = vec![Check { kind: Kind::File, outcome: Outcome::Passed }];

    let Some((host, port)) = remote_of(&text) else {
        checks.push(Check { kind: Kind::Remote, outcome: Outcome::Failed(String::new()) });
        return checks;
    };
    checks.push(Check { kind: Kind::Remote, outcome: Outcome::Passed });

    let missing = missing_files(&text, profile.workdir.as_deref());
    checks.push(Check {
        kind: Kind::Files,
        outcome: if missing.is_empty() { Outcome::Passed } else { Outcome::Failed(missing.join(", ")) },
    });

    let addrs: Vec<SocketAddr> = match (host.as_str(), port).to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            checks.push(Check { kind: Kind::Dns, outcome: Outcome::Failed(e.to_string()) });
            return checks;
        }
    };
    if addrs.is_empty() {
        checks.push(Check { kind: Kind::Dns, outcome: Outcome::Failed(host) });
        return checks;
    }
    checks.push(Check { kind: Kind::Dns, outcome: Outcome::Passed });

    checks.push(port_check(&text, &addrs));
    checks
}

/// The first `remote host port` line. OpenVPN defaults the port to 1194.
fn remote_of(text: &str) -> Option<(String, u16)> {
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("remote") {
            continue;
        }
        let Some(host) = words.next() else { continue };
        let port = words.next().and_then(|p| p.parse().ok()).unwrap_or(1194);
        return Some((host.to_string(), port));
    }
    None
}

/// The profile's `proto`, which decides whether a TCP probe means anything.
fn proto_of(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("proto")).then(|| words.next()).flatten()
        })
        .unwrap_or("udp")
        .to_ascii_lowercase()
}

fn missing_files(text: &str, workdir: Option<&Path>) -> Vec<String> {
    let base = workdir.unwrap_or(Path::new("."));
    let mut missing: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(directive) = words.next() else { continue };
        if !FILE_DIRECTIVES.contains(&directive) {
            continue;
        }
        let Some(name) = words.next() else { continue };
        if name.starts_with('<') || name.starts_with('[') {
            continue;
        }
        let path = if Path::new(name).is_absolute() { PathBuf::from(name) } else { base.join(name) };
        if !path.exists() && !missing.iter().any(|m| m == name) {
            missing.push(name.to_string());
        }
    }
    missing
}

fn port_check(text: &str, addrs: &[SocketAddr]) -> Check {
    if proto_of(text).starts_with("udp") {
        return Check { kind: Kind::Port, outcome: Outcome::Skipped("udp".to_string()) };
    }
    let mut last = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(addr, TIMEOUT) {
            Ok(_) => return Check { kind: Kind::Port, outcome: Outcome::Passed },
            Err(e) => last = e.to_string(),
        }
    }
    Check { kind: Kind::Port, outcome: Outcome::Failed(last) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(dir: &Path, body: &str) -> Profile {
        let file = dir.join("test.ovpn");
        std::fs::write(&file, body).unwrap();
        Profile {
            name: "test".into(),
            file,
            workdir: Some(dir.to_path_buf()),
            username: String::new(),
            saved_password: false,
            remote: String::new(),
            last_used: 0,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ether-ovpn-preflight-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_the_remote_line() {
        assert_eq!(remote_of("client\nremote vpn.example.com 1195\n"), Some(("vpn.example.com".into(), 1195)));
        // OpenVPN's default port.
        assert_eq!(remote_of("remote vpn.example.com\n"), Some(("vpn.example.com".into(), 1194)));
        assert_eq!(remote_of("client\n"), None);
    }

    #[test]
    fn reads_the_protocol() {
        assert_eq!(proto_of("client\nproto tcp\n"), "tcp");
        assert_eq!(proto_of("client\nproto udp4\n"), "udp4");
        // OpenVPN defaults to UDP.
        assert_eq!(proto_of("client\n"), "udp");
    }

    #[test]
    fn finds_missing_files_and_ignores_inline_blocks() {
        let dir = temp("files");
        std::fs::write(dir.join("ca.crt"), "x").unwrap();
        let text = "client\nca ca.crt\ncert missing.crt\n<ca>\n-----BEGIN-----\n</ca>\nauth-user-pass\n";
        assert_eq!(missing_files(text, Some(&dir)), vec!["missing.crt"]);
    }

    #[test]
    fn reports_a_profile_without_a_remote() {
        let dir = temp("noremote");
        let checks = run(&profile(&dir, "client\ndev tun\n"));
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[1].kind, Kind::Remote);
        assert!(matches!(checks[1].outcome, Outcome::Failed(_)));
    }

    #[test]
    fn reports_a_missing_certificate() {
        let dir = temp("nocert");
        let checks = run(&profile(&dir, "client\nremote 127.0.0.1 1194\nproto tcp\nca absent.crt\n"));
        let files = checks.iter().find(|c| c.kind == Kind::Files).unwrap();
        assert_eq!(files.outcome, Outcome::Failed("absent.crt".into()));
    }

    #[test]
    fn skips_the_port_probe_for_udp() {
        let dir = temp("udp");
        let checks = run(&profile(&dir, "client\nremote 127.0.0.1 1194\nproto udp\n"));
        let port = checks.iter().find(|c| c.kind == Kind::Port).unwrap();
        assert_eq!(port.outcome, Outcome::Skipped("udp".into()));
    }

    #[test]
    fn reports_an_unreadable_profile() {
        let dir = temp("gone");
        let mut p = profile(&dir, "client\n");
        p.file = dir.join("absent.ovpn");
        let checks = run(&p);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].kind, Kind::File);
        assert!(matches!(checks[0].outcome, Outcome::Failed(_)));
    }
}
