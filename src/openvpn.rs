//! Runs `openvpn` (2.x) through pkexec and drives it over its management interface.
//!
//! The wire format, including the challenge/response variants, is documented in
//! OpenVPN's own management-notes.txt:
//! <https://github.com/OpenVPN/openvpn/blob/master/doc/management-notes.txt>

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

pub enum Event {
    State {
        name: String,
        local_ip: String,
        remote: String,
    },
    Bytes {
        rx: u64,
        tx: u64,
    },
    Log(String),
    NeedAuth(AuthRequest),
    /// The server wants a challenge response outside the password prompt (crtext).
    Challenge(Challenge),
    /// Authentication is waiting on the user, and the server may time it out.
    AuthPending {
        timeout: Option<u32>,
    },
    AuthFailed,
    /// The openvpn process is gone. Carries an error message unless it was a clean stop.
    Exited(Option<String>),
    /// The management connection is gone while openvpn is still running, so
    /// Ether can no longer control the tunnel.
    Lost,
}

#[derive(Clone, Debug)]
pub struct AuthRequest {
    /// The name openvpn asked for, e.g. "Auth" or "Private Key".
    pub realm: String,
    pub needs_username: bool,
    /// Set when the server wants a challenge response along with the password.
    pub challenge: Option<Challenge>,
}

/// A challenge/response the server asked for on top of the password.
#[derive(Clone, Debug, PartialEq)]
pub enum Challenge {
    /// `--static-challenge` in the profile: the response travels with the password.
    Static { echo: bool, concat: bool, text: String },
    /// CRV1: the previous attempt failed, and the response goes in the next password.
    Dynamic {
        echo: bool,
        required: bool,
        state: String,
        text: String,
    },
    /// crtext (OpenVPN 2.5+): answered with `cr-response` while auth is pending.
    CrText { echo: bool, required: bool, text: String },
}

type Emit = Arc<dyn Fn(Event) + Send + Sync>;

struct Shared {
    mgmt: Mutex<Option<TcpStream>>,
    stop_requested: AtomicBool,
    exited: AtomicBool,
    /// Set when the management connection is gone for good, so a stop request
    /// has nowhere to go.
    lost: AtomicBool,
}

pub struct Session {
    shared: Arc<Shared>,
}

impl Session {
    pub fn start(
        config: &Path,
        workdir: Option<&Path>,
        saved_password: bool,
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
        let workdir = workdir.or(config.parent()).unwrap_or(Path::new("/")).to_path_buf();

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
            // Announce crtext support, or servers that check IV_SSO never send a
            // 2FA challenge (OpenVPN 2.5+).
            .args(["--setenv", "IV_SSO", "crtext"])
            .args(dns_script_args())
            .args(privilege_args(config))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The log is the main diagnostic tool, so keep it informative whatever
        // the profile asks for. Later options win, so this overrides `verb`.
        cmd.args(["--verb", "3"]);
        if saved_password {
            // The keyring can hand the password over again, so openvpn doesn't
            // have to keep it in memory across renegotiations. Without a saved
            // password this would turn every renegotiation into a new prompt.
            cmd.arg("--auth-nocache");
        }
        let mut child = cmd.spawn().map_err(|e| {
            let _ = fs::remove_file(&pw_file);
            io::Error::new(e.kind(), format!("could not run pkexec: {e}"))
        })?;

        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            lost: AtomicBool::new(false),
        });

        // Both pipes were asked for above, so they are always there.
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

    pub fn send_auth(&self, req: &AuthRequest, username: &str, password: &str, code: &str) {
        let realm = quote(&req.realm);
        if req.needs_username {
            self.send(&format!("username {realm} {}", quote(username)));
        }
        let password = match &req.challenge {
            // FORMAT=1: the profile asked for the response to be glued to the password.
            Some(Challenge::Static { concat: true, .. }) => format!("{password}{code}"),
            Some(Challenge::Static { .. }) => format!("SCRV1:{}:{}", base64(password), base64(code)),
            Some(Challenge::Dynamic { state, .. }) => format!("CRV1::{state}::{code}"),
            _ => password.to_string(),
        };
        self.send(&format!("password {realm} {}", quote(&password)));
    }

    /// Answers a crtext challenge, which arrives while authentication is pending.
    pub fn send_cr_response(&self, code: &str) {
        self.send(&format!("cr-response {}", base64(code)));
    }

    /// Asks openvpn to stop. Returns false when the management connection is
    /// gone, so the caller can say so instead of pretending it worked.
    ///
    /// While the connection is still being set up the request is honoured as
    /// soon as it is, so only a lost connection counts as a failure.
    pub fn disconnect(&self) -> bool {
        self.shared.stop_requested.store(true, Ordering::SeqCst);
        if self.shared.lost.load(Ordering::SeqCst) {
            return false;
        }
        self.send("signal SIGTERM");
        true
    }

    fn send(&self, line: &str) {
        send(&self.shared, line);
    }
}

/// The management stream, ignoring a poisoned lock: it only ever guards a
/// socket handle, and a panic elsewhere must not take the tunnel down with it.
fn mgmt(shared: &Shared) -> MutexGuard<'_, Option<TcpStream>> {
    shared.mgmt.lock().unwrap_or_else(|e| e.into_inner())
}

fn send(shared: &Shared, line: &str) {
    if let Some(stream) = mgmt(shared).as_mut() {
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
    *mgmt(shared) = Some(writer);

    send(shared, "state on");
    send(shared, "bytecount 2");
    if shared.stop_requested.load(Ordering::SeqCst) {
        send(shared, "signal SIGTERM");
    } else {
        send(shared, "hold release");
    }

    // A CRV1 challenge is answered through the password prompt that follows it.
    let mut pending: Option<Challenge> = None;

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        // The password prompt has no newline, so it ends up glued to the first reply.
        let line = line.trim_end_matches('\r').trim_start_matches("ENTER PASSWORD:");
        let Some(msg) = line.strip_prefix('>') else { continue };
        if msg.starts_with("HOLD:") {
            send(shared, "hold release");
        } else if let Some(event) = parse_message(msg) {
            match event {
                Event::Challenge(c @ Challenge::Dynamic { .. }) => pending = Some(c),
                Event::NeedAuth(mut req) => {
                    if req.challenge.is_none() {
                        req.challenge = pending.take();
                    }
                    emit(Event::NeedAuth(req));
                }
                other => emit(other),
            }
        }
    }
    *mgmt(shared) = None;
    // openvpn keeps running when its management client goes away, so the
    // tunnel may still be up while Ether can no longer control it.
    if !shared.exited.load(Ordering::SeqCst) && !shared.stop_requested.load(Ordering::SeqCst) {
        shared.lost.store(true, Ordering::SeqCst);
        emit(Event::Lost);
    }
}

fn parse_message(msg: &str) -> Option<Event> {
    if let Some(rest) = msg.strip_prefix("STATE:") {
        // time,state,description,local ip,remote ip,remote port,...
        let f: Vec<&str> = rest.split(',').collect();
        let field = |i: usize| f.get(i).copied().unwrap_or("").to_string();
        // AUTH_PENDING carries the server's timeout in the description field.
        if field(1) == "AUTH_PENDING" {
            return Some(Event::AuthPending {
                timeout: field(2).parse().ok(),
            });
        }
        let remote = match (field(4), field(5)) {
            (ip, _) if ip.is_empty() => String::new(),
            (ip, port) if port.is_empty() => ip,
            (ip, port) => format!("{ip}:{port}"),
        };
        return Some(Event::State {
            name: field(1),
            local_ip: field(3),
            remote,
        });
    }
    if let Some(rest) = msg.strip_prefix("BYTECOUNT:") {
        let (rx, tx) = rest.split_once(',')?;
        return Some(Event::Bytes {
            rx: rx.trim().parse().ok()?,
            tx: tx.trim().parse().ok()?,
        });
    }
    if let Some(rest) = msg.strip_prefix("INFOMSG:") {
        // CR_TEXT:<flags>:<challenge_text>, the crtext challenge of OpenVPN 2.5+.
        let (flags, text) = rest.strip_prefix("CR_TEXT:")?.split_once(':')?;
        return Some(Event::Challenge(Challenge::CrText {
            echo: has_flag(flags, 'E'),
            required: has_flag(flags, 'R'),
            text: text.to_string(),
        }));
    }
    if let Some(rest) = msg.strip_prefix("PASSWORD:") {
        if let Some(rest) = rest.strip_prefix("Verification Failed") {
            // 'Auth' ['CRV1:<flags>:<state_id>:<username_base64>:<challenge_text>']
            if let Some((_, crv1)) = rest.split_once("['CRV1:") {
                return parse_crv1(crv1.trim_end_matches("']")).map(Event::Challenge);
            }
            return Some(Event::AuthFailed);
        }
        let rest = rest.strip_prefix("Need '")?;
        let (realm, tail) = rest.split_once('\'')?;
        return Some(Event::NeedAuth(AuthRequest {
            realm: realm.to_string(),
            needs_username: tail.trim_start().starts_with("username/password"),
            challenge: parse_static_challenge(tail),
        }));
    }
    if let Some(rest) = msg.strip_prefix("FATAL:") {
        return Some(Event::Log(format!("FATAL: {rest}")));
    }
    None
}

/// Whether a comma-separated flag list such as `R,E` contains `flag`.
fn has_flag(flags: &str, flag: char) -> bool {
    flags.split(',').any(|f| f.trim() == flag.to_string())
}

/// `SC:<flag>,<text>` at the end of a password request. The low bit of the flag
/// asks for the response to be echoed, the next bit asks for plain concatenation
/// instead of the SCRV1 encoding.
fn parse_static_challenge(tail: &str) -> Option<Challenge> {
    let (_, rest) = tail.split_once("SC:")?;
    let (flag, text) = rest.split_once(',')?;
    let flag: u32 = flag.trim().parse().ok()?;
    Some(Challenge::Static {
        echo: flag & 1 != 0,
        concat: flag & 2 != 0,
        text: text.to_string(),
    })
}

/// `<flags>:<state_id>:<username_base64>:<challenge_text>`, where the text may
/// itself contain colons.
fn parse_crv1(rest: &str) -> Option<Challenge> {
    let mut parts = rest.splitn(4, ':');
    let flags = parts.next()?;
    let state = parts.next()?;
    let _username = parts.next()?;
    let text = parts.next()?;
    Some(Challenge::Dynamic {
        echo: has_flag(flags, 'E'),
        required: has_flag(flags, 'R'),
        state: state.to_string(),
        text: text.to_string(),
    })
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

/// Base64, as the challenge/response protocol wants passwords and codes encoded.
fn base64(data: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = data.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
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
    ["/usr/sbin/openvpn", "/usr/bin/openvpn"]
        .into_iter()
        .find(|p| on_host(p).exists())
}

/// Hooks up the distro's helper script so DNS servers pushed by the gateway are applied.
fn dns_script_args() -> Vec<String> {
    let resolved = "/etc/openvpn/update-systemd-resolved";
    let resolvconf = "/etc/openvpn/update-resolv-conf";
    let uses_resolved = fs::read_to_string(on_host("/etc/resolv.conf")).is_ok_and(|s| s.contains("127.0.0.53"));
    let has_resolvconf = ["/usr/sbin/resolvconf", "/sbin/resolvconf"]
        .into_iter()
        .any(|p| on_host(p).exists());
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

/// Profiles made on Debian often say `group nogroup`, which other distros call
/// `nobody`. OpenVPN only drops privileges after signing in, so a missing name
/// fails the connection then. Later options win, so these replace the profile's.
fn privilege_args(config: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(config) else {
        return Vec::new();
    };
    let names = |file| -> Vec<String> {
        fs::read_to_string(on_host(file))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split(':').next())
            .map(str::to_string)
            .collect()
    };
    privilege_overrides(&text, &names("/etc/passwd"), &names("/etc/group"))
}

fn privilege_overrides(config: &str, users: &[String], groups: &[String]) -> Vec<String> {
    let directive = |name: &str| {
        config.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(name)).then(|| words.next()).flatten()
        })
    };
    let mut args = Vec::new();
    for (option, known, fallbacks) in [
        ("user", users, ["nobody"].as_slice()),
        ("group", groups, &["nogroup", "nobody"]),
    ] {
        let Some(wanted) = directive(option) else { continue };
        if known.iter().any(|n| n == wanted) {
            continue;
        }
        if let Some(fallback) = fallbacks.iter().find(|f| known.iter().any(|n| n == *f)) {
            args.extend([format!("--{option}"), fallback.to_string()]);
        }
    }
    args
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
    let dir = private_dir(&dir.join("ether"))?;
    let path = dir.join(format!("mgmt-{port}.pw"));
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    writeln!(f, "{secret}")?;
    Ok(path)
}

/// Creates `dir` with mode 0700, refusing anything that isn't a real folder.
///
/// The fallback when XDG_RUNTIME_DIR is unset is a shared temporary folder, so a
/// symlink planted there would otherwise hand the management password to another
/// local user. `set_permissions` also fails if the folder belongs to someone else.
fn private_dir(dir: &Path) -> io::Result<PathBuf> {
    match fs::create_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} isn't a folder", dir.display()),
        ));
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir.to_path_buf())
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

        assert!(matches!(
            parse_message("PASSWORD:Verification Failed: 'Auth'"),
            Some(Event::AuthFailed)
        ));
    }

    #[test]
    fn parses_bytecount() {
        assert!(matches!(
            parse_message("BYTECOUNT:1024,2048"),
            Some(Event::Bytes { rx: 1024, tx: 2048 })
        ));
    }

    /// The line the TimeWeb server actually sends, taken from a real connection log.
    #[test]
    fn parses_static_challenge() {
        let Some(Event::NeedAuth(req)) = parse_message("PASSWORD:Need 'Auth' username/password SC:1,Enter code") else {
            panic!("expected auth request");
        };
        assert_eq!(req.realm, "Auth");
        assert!(req.needs_username);
        assert_eq!(
            req.challenge,
            Some(Challenge::Static {
                echo: true,
                concat: false,
                text: "Enter code".to_string()
            })
        );

        // FORMAT=1 asks for the response to be glued to the password instead.
        let Some(Event::NeedAuth(req)) = parse_message("PASSWORD:Need 'Auth' username/password SC:2,Enter code") else {
            panic!("expected auth request");
        };
        assert_eq!(
            req.challenge,
            Some(Challenge::Static {
                echo: false,
                concat: true,
                text: "Enter code".to_string()
            })
        );

        // A preferred username may precede the challenge.
        let Some(Event::NeedAuth(req)) =
            parse_message("PASSWORD:Need 'Auth' username/password UN:Y3Ix SC:1,Please enter token PIN")
        else {
            panic!("expected auth request");
        };
        assert_eq!(
            req.challenge,
            Some(Challenge::Static {
                echo: true,
                concat: false,
                text: "Please enter token PIN".to_string()
            })
        );
    }

    #[test]
    fn parses_dynamic_challenge() {
        let Some(Event::Challenge(Challenge::Dynamic {
            echo,
            required,
            state,
            text,
        })) = parse_message(
            "PASSWORD:Verification Failed: 'Auth' ['CRV1:R,E:Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l:Y3Ix:Please enter token PIN']",
        )
        else {
            panic!("expected a dynamic challenge");
        };
        assert!(echo);
        assert!(required);
        assert_eq!(state, "Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l");
        assert_eq!(text, "Please enter token PIN");

        // The challenge text may contain colons.
        let Some(Event::Challenge(Challenge::Dynamic { text, .. })) =
            parse_message("PASSWORD:Verification Failed: 'Auth' ['CRV1:R:abc:Y3Ix:Code: 123456']")
        else {
            panic!("expected a dynamic challenge");
        };
        assert_eq!(text, "Code: 123456");

        // Without a CRV1 payload it stays a plain failure.
        assert!(matches!(
            parse_message("PASSWORD:Verification Failed: 'Auth'"),
            Some(Event::AuthFailed)
        ));
    }

    #[test]
    fn parses_crtext_challenge() {
        let Some(Event::Challenge(Challenge::CrText { echo, required, text })) =
            parse_message("INFOMSG:CR_TEXT:R,E:Please enter token PIN")
        else {
            panic!("expected a crtext challenge");
        };
        assert!(echo);
        assert!(required);
        assert_eq!(text, "Please enter token PIN");

        assert!(parse_message("INFOMSG:NOTIFY::Checking your account").is_none());
    }

    #[test]
    fn parses_auth_pending() {
        assert!(matches!(
            parse_message("STATE:1794353029,AUTH_PENDING,60,,,,,"),
            Some(Event::AuthPending { timeout: Some(60) })
        ));
        assert!(matches!(
            parse_message("STATE:1794353029,AUTH_PENDING,,,,,,"),
            Some(Event::AuthPending { timeout: None })
        ));
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(""), "");
        assert_eq!(base64("f"), "Zg==");
        assert_eq!(base64("fo"), "Zm8=");
        assert_eq!(base64("foo"), "Zm9v");
        // The examples from the management interface documentation.
        assert_eq!(base64("bar"), "YmFy");
        assert_eq!(base64("8675309"), "ODY3NTMwOQ==");
    }

    #[test]
    fn builds_challenge_responses() {
        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            lost: AtomicBool::new(false),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        *shared.mgmt.lock().unwrap() = Some(stream);
        let session = Session { shared };
        let mut input = BufReader::new(client).lines().map(Result::unwrap);

        let static_req = AuthRequest {
            realm: "Auth".into(),
            needs_username: true,
            challenge: Some(Challenge::Static {
                echo: true,
                concat: false,
                text: "Enter code".into(),
            }),
        };
        session.send_auth(&static_req, "foo", "bar", "8675309");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "foo""#);
        assert_eq!(input.next().unwrap(), r#"password "Auth" "SCRV1:YmFy:ODY3NTMwOQ==""#);

        let concat_req = AuthRequest {
            realm: "Auth".into(),
            needs_username: true,
            challenge: Some(Challenge::Static {
                echo: false,
                concat: true,
                text: "Enter code".into(),
            }),
        };
        session.send_auth(&concat_req, "foo", "bar", "8675309");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "foo""#);
        assert_eq!(input.next().unwrap(), r#"password "Auth" "bar8675309""#);

        let dynamic_req = AuthRequest {
            realm: "Auth".into(),
            needs_username: true,
            challenge: Some(Challenge::Dynamic {
                echo: true,
                required: true,
                state: "Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l".into(),
                text: "Please enter token PIN".into(),
            }),
        };
        session.send_auth(&dynamic_req, "cr1", "bar", "8675309");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "cr1""#);
        assert_eq!(
            input.next().unwrap(),
            r#"password "Auth" "CRV1::Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l::8675309""#
        );

        session.send_cr_response("8675309");
        assert_eq!(input.next().unwrap(), "cr-response ODY3NTMwOQ==");

        // Without a challenge the password goes out untouched.
        let plain = AuthRequest {
            realm: "Auth".into(),
            needs_username: true,
            challenge: None,
        };
        session.send_auth(&plain, "foo", "bar", "");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "foo""#);
        assert_eq!(input.next().unwrap(), r#"password "Auth" "bar""#);
    }

    /// openvpn keeps running when its management client goes away, so the app
    /// has to hear about it instead of showing a connection it can't control.
    #[test]
    fn reports_a_lost_management_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            lost: AtomicBool::new(false),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let emit: Emit = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let client = {
            let shared = shared.clone();
            thread::spawn(move || run_management(port, "s3cret", &shared, &emit))
        };

        // Play the openvpn side, then vanish without being asked to stop.
        let (stream, _) = listener.accept().unwrap();
        let mut input = BufReader::new(stream.try_clone().unwrap()).lines().map(Result::unwrap);
        assert_eq!(input.next().unwrap(), "s3cret");
        drop(input);
        drop(stream);
        client.join().unwrap();

        let events: Vec<Event> = rx.try_iter().collect();
        assert!(events.iter().any(|e| matches!(e, Event::Lost)), "expected a lost event");
    }

    /// A stop request has nowhere to go once the management connection is gone,
    /// so the caller has to hear about it instead of showing "disconnecting".
    #[test]
    fn disconnect_reports_a_lost_connection() {
        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            lost: AtomicBool::new(false),
        });
        let session = Session { shared: shared.clone() };
        assert!(session.disconnect());
        assert!(shared.stop_requested.load(Ordering::SeqCst));

        shared.lost.store(true, Ordering::SeqCst);
        assert!(!session.disconnect());
    }

    #[test]
    fn talks_to_management_interface() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Shared {
            mgmt: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            lost: AtomicBool::new(false),
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
        out.write_all(b"SUCCESS: password is correct\r\n>HOLD:Waiting for hold release:0\r\n")
            .unwrap();
        assert_eq!(input.next().unwrap(), "state on");
        assert_eq!(input.next().unwrap(), "bytecount 2");
        assert_eq!(input.next().unwrap(), "hold release");
        assert_eq!(input.next().unwrap(), "hold release");
        out.write_all(b">PASSWORD:Need 'Auth' username/password\r\n").unwrap();
        let Event::NeedAuth(req) = rx.recv().unwrap() else {
            panic!("expected auth request")
        };

        let session = Session { shared: shared.clone() };
        session.send_auth(&req, "bryan", "p\"w", "");
        assert_eq!(input.next().unwrap(), r#"username "Auth" "bryan""#);
        assert_eq!(input.next().unwrap(), r#"password "Auth" "p\"w""#);

        out.write_all(b">STATE:1,CONNECTED,SUCCESS,192.168.3.2,198.51.100.7,1194,,\r\n")
            .unwrap();
        assert!(matches!(rx.recv().unwrap(), Event::State { name, .. } if name == "CONNECTED"));

        assert!(session.disconnect());
        assert_eq!(input.next().unwrap(), "signal SIGTERM");
        drop(out);
        drop(input);
        client.join().unwrap();
    }

    #[test]
    fn quotes_values() {
        assert_eq!(quote(r#"pa"ss\word"#), r#""pa\"ss\\word""#);
    }

    #[test]
    fn replaces_missing_user_and_group() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let config = "client\nuser nobody\ngroup nogroup\n";
        // Arch and Fedora have no nogroup.
        assert_eq!(
            privilege_overrides(config, &names(&["root", "nobody"]), &names(&["root", "nobody"])),
            ["--group", "nobody"]
        );
        // Debian has both names the profile asks for.
        assert!(privilege_overrides(config, &names(&["nobody"]), &names(&["nogroup"])).is_empty());
        assert!(privilege_overrides("client\n", &[], &[]).is_empty());
    }
}
