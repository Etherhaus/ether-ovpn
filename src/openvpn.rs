//! Runs `openvpn` (2.x) through pkexec and drives it over its management interface.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub enum Event {
    State { name: String, local_ip: String, remote: String },
    Bytes { rx: u64, tx: u64 },
    Log(String),
    NeedAuth(AuthRequest),
    AuthFailed,
    /// The openvpn process is gone. Carries an error message unless it was a clean stop.
    Exited(Option<String>),
}

#[derive(Clone, Debug)]
pub struct AuthRequest {
    /// The name openvpn asked for, e.g. "Auth" or "Private Key".
    pub realm: String,
    pub needs_username: bool,
}

type Emit = Arc<dyn Fn(Event) + Send + Sync>;

struct Shared {
    mgmt: Mutex<Option<TcpStream>>,
    stop_requested: AtomicBool,
    exited: AtomicBool,
}

pub struct Session {
    shared: Arc<Shared>,
}

impl Session {
    pub fn start(
        config: &Path,
        workdir: Option<&Path>,
        emit: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Session> {
        let emit: Emit = Arc::new(emit);
        let openvpn = find_openvpn().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "openvpn is not installed (sudo apt install openvpn)",
            )
        })?;
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let secret = random_hex(16)?;
        let pw_file = write_secret(port, &secret)?;
        let workdir = workdir
            .or(config.parent())
            .unwrap_or(Path::new("/"))
            .to_path_buf();

        let mut cmd = if crate::sandboxed() {
            let mut cmd = Command::new("flatpak-spawn");
            cmd.args(["--host", "pkexec"]);
            cmd
        } else {
            Command::new("pkexec")
        };
        cmd.arg(openvpn)
            // --cd must come before --config so relative paths inside the profile resolve.
            .arg("--cd")
            .arg(&workdir)
            .arg("--config")
            .arg(config)
            .args(["--management", "127.0.0.1", &port.to_string()])
            .arg(&pw_file)
            .args(["--management-hold", "--management-query-passwords"])
            .args(["--auth-retry", "interact"])
            .args(dns_script_args())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            let _ = fs::remove_file(&pw_file);
            io::Error::new(e.kind(), format!("could not run pkexec: {e}"))
        })?;

        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });

        let readers = [
            spawn_log_reader(child.stdout.take().unwrap(), emit.clone()),
            spawn_log_reader(child.stderr.take().unwrap(), emit.clone()),
        ];

        {
            let shared = shared.clone();
            let emit = emit.clone();
            thread::spawn(move || {
                let status = child.wait();
                shared.exited.store(true, Ordering::SeqCst);
                for r in readers {
                    let _ = r.join();
                }
                let _ = fs::remove_file(&pw_file);
                let message = match status {
                    _ if shared.stop_requested.load(Ordering::SeqCst) => None,
                    Ok(s) => match s.code() {
                        Some(0) | None => None,
                        Some(126) => Some("Authorization was cancelled.".to_string()),
                        Some(127) => Some("Not authorized to start OpenVPN.".to_string()),
                        Some(code) => Some(format!("OpenVPN stopped with an error (code {code}). See the log.")),
                    },
                    Err(e) => Some(format!("Lost track of OpenVPN: {e}")),
                };
                emit(Event::Exited(message));
            });
        }

        {
            let shared = shared.clone();
            thread::spawn(move || run_management(port, &secret, &shared, &emit));
        }

        Ok(Session { shared })
    }

    pub fn send_auth(&self, req: &AuthRequest, username: &str, password: &str) {
        let realm = quote(&req.realm);
        if req.needs_username {
            self.send(&format!("username {realm} {}", quote(username)));
        }
        self.send(&format!("password {realm} {}", quote(password)));
    }

    pub fn disconnect(&self) {
        self.shared.stop_requested.store(true, Ordering::SeqCst);
        self.send("signal SIGTERM");
    }

    fn send(&self, line: &str) {
        send(&self.shared, line);
    }
}

fn send(shared: &Shared, line: &str) {
    if let Some(stream) = shared.mgmt.lock().unwrap().as_mut() {
        let _ = stream.write_all(format!("{line}\n").as_bytes());
    }
}

fn run_management(port: u16, secret: &str, shared: &Shared, emit: &Emit) {
    // openvpn only starts listening after the user has answered the pkexec prompt.
    let stream = loop {
        if shared.exited.load(Ordering::SeqCst) {
            return;
        }
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    };
    let Ok(mut writer) = stream.try_clone() else { return };
    let _ = writer.write_all(format!("{secret}\n").as_bytes());
    *shared.mgmt.lock().unwrap() = Some(writer);

    send(shared, "state on");
    send(shared, "bytecount 2");
    if shared.stop_requested.load(Ordering::SeqCst) {
        send(shared, "signal SIGTERM");
    } else {
        send(shared, "hold release");
    }

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        // The password prompt has no newline, so it ends up glued to the first reply.
        let line = line.trim_end_matches('\r').trim_start_matches("ENTER PASSWORD:");
        let Some(msg) = line.strip_prefix('>') else { continue };
        if msg.starts_with("HOLD:") {
            send(shared, "hold release");
        } else if let Some(event) = parse_message(msg) {
            emit(event);
        }
    }
    *shared.mgmt.lock().unwrap() = None;
}

fn parse_message(msg: &str) -> Option<Event> {
    if let Some(rest) = msg.strip_prefix("STATE:") {
        // time,state,description,local ip,remote ip,remote port,...
        let f: Vec<&str> = rest.split(',').collect();
        let field = |i: usize| f.get(i).copied().unwrap_or("").to_string();
        let remote = match (field(4), field(5)) {
            (ip, _) if ip.is_empty() => String::new(),
            (ip, port) if port.is_empty() => ip,
            (ip, port) => format!("{ip}:{port}"),
        };
        return Some(Event::State { name: field(1), local_ip: field(3), remote });
    }
    if let Some(rest) = msg.strip_prefix("BYTECOUNT:") {
        let (rx, tx) = rest.split_once(',')?;
        return Some(Event::Bytes { rx: rx.trim().parse().ok()?, tx: tx.trim().parse().ok()? });
    }
    if let Some(rest) = msg.strip_prefix("PASSWORD:") {
        if rest.starts_with("Verification Failed") {
            return Some(Event::AuthFailed);
        }
        let rest = rest.strip_prefix("Need '")?;
        let (realm, tail) = rest.split_once('\'')?;
        return Some(Event::NeedAuth(AuthRequest {
            realm: realm.to_string(),
            needs_username: tail.trim_start().starts_with("username/password"),
        }));
    }
    if let Some(rest) = msg.strip_prefix("FATAL:") {
        return Some(Event::Log(format!("FATAL: {rest}")));
    }
    None
}

fn spawn_log_reader(pipe: impl Read + Send + 'static, emit: Emit) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            emit(Event::Log(line));
        }
    })
}

/// Quotes a value for the management protocol, which escapes `\` and `"` with a backslash.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().filter(|c| *c != '\n' && *c != '\r') {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Where a host path can be read from here: the same path natively, or under
/// /run/host inside the Flatpak sandbox.
fn on_host(path: &str) -> PathBuf {
    if crate::sandboxed() {
        Path::new("/run/host").join(path.trim_start_matches('/'))
    } else {
        PathBuf::from(path)
    }
}

fn find_openvpn() -> Option<&'static str> {
    ["/usr/sbin/openvpn", "/usr/bin/openvpn"].into_iter().find(|p| on_host(p).exists())
}

/// Hooks up the distro's helper script so DNS servers pushed by the gateway are applied.
fn dns_script_args() -> Vec<String> {
    let resolved = "/etc/openvpn/update-systemd-resolved";
    let resolvconf = "/etc/openvpn/update-resolv-conf";
    let uses_resolved = fs::read_to_string(on_host("/etc/resolv.conf")).is_ok_and(|s| s.contains("127.0.0.53"));
    let has_resolvconf = ["/usr/sbin/resolvconf", "/sbin/resolvconf"].into_iter().any(|p| on_host(p).exists());
    let script = if on_host(resolved).exists() && uses_resolved {
        resolved
    } else if on_host(resolvconf).exists() && has_resolvconf {
        resolvconf
    } else {
        return Vec::new();
    };
    let script = script.to_string();
    vec![
        "--script-security".into(),
        "2".into(),
        "--up".into(),
        script.clone(),
        "--up-restart".into(),
        "--down".into(),
        script,
        "--down-pre".into(),
    ]
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn write_secret(port: u16, secret: &str) -> io::Result<PathBuf> {
    let mut dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    // Inside Flatpak only the app's own runtime folder is shared with the host.
    if let Ok(app_id) = std::env::var("FLATPAK_ID") {
        dir = dir.join("app").join(app_id);
    }
    let dir = dir.join("vpndeck");
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let path = dir.join(format!("mgmt-{port}.pw"));
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    writeln!(f, "{secret}")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state() {
        let Some(Event::State { name, local_ip, remote }) =
            parse_message("STATE:1730000000,CONNECTED,SUCCESS,10.8.0.2,203.0.113.5,1194,,")
        else {
            panic!("expected state");
        };
        assert_eq!(name, "CONNECTED");
        assert_eq!(local_ip, "10.8.0.2");
        assert_eq!(remote, "203.0.113.5:1194");
    }

    #[test]
    fn parses_password_prompts() {
        let Some(Event::NeedAuth(req)) = parse_message("PASSWORD:Need 'Auth' username/password") else {
            panic!("expected auth request");
        };
        assert_eq!(req.realm, "Auth");
        assert!(req.needs_username);

        let Some(Event::NeedAuth(req)) = parse_message("PASSWORD:Need 'Private Key' password") else {
            panic!("expected key request");
        };
        assert_eq!(req.realm, "Private Key");
        assert!(!req.needs_username);

        assert!(matches!(parse_message("PASSWORD:Verification Failed: 'Auth'"), Some(Event::AuthFailed)));
    }

    #[test]
    fn parses_bytecount() {
        assert!(matches!(parse_message("BYTECOUNT:1024,2048"), Some(Event::Bytes { rx: 1024, tx: 2048 })));
    }

    #[test]
    fn talks_to_management_interface() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let emit: Emit = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let client = {
            let shared = shared.clone();
            thread::spawn(move || run_management(port, "s3cret", &shared, &emit))
        };

        // Play the openvpn side.
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut input = BufReader::new(stream).lines().map(Result::unwrap);
        out.write_all(b"ENTER PASSWORD:").unwrap();
        assert_eq!(input.next().unwrap(), "s3cret");
        out.write_all(b"SUCCESS: password is correct\r\n>HOLD:Waiting for hold release:0\r\n").unwrap();
        assert_eq!(input.next().unwrap(), "state on");
        assert_eq!(input.next().unwrap(), "bytecount 2");
        assert_eq!(input.next().unwrap(), "hold release");
        assert_eq!(input.next().unwrap(), "hold release");
        out.write_all(b">PASSWORD:Need 'Auth' username/password\r\n").unwrap();
        let Event::NeedAuth(req) = rx.recv().unwrap() else { panic!("expected auth request") };

        let session = Session { shared: shared.clone() };
        session.send_auth(&req, "bryan", "p\"w");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "bryan""#);
        assert_eq!(input.next().unwrap(), r#"password "Auth" "p\"w""#);

        out.write_all(b">STATE:1,CONNECTED,SUCCESS,192.168.3.2,198.51.100.7,1194,,\r\n").unwrap();
        assert!(matches!(rx.recv().unwrap(), Event::State { name, .. } if name == "CONNECTED"));

        session.disconnect();
        assert_eq!(input.next().unwrap(), "signal SIGTERM");
        drop(out);
        drop(input);
        client.join().unwrap();
    }

    #[test]
    fn quotes_values() {
        assert_eq!(quote(r#"pa"ss\word"#), r#""pa\"ss\\word""#);
    }
}
