//! Checks GitHub for a newer Ether release, and downloads and installs it.
//!
//! A standalone binary (release tarball) is replaced in place, the .deb and
//! .rpm are installed through apt, dnf or zypper (asking for the admin
//! password), and the Flatpak is reinstalled through the host's `flatpak` command.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// The GitHub repository releases are published to, from `Cargo.toml`.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const CHECKSUMS: &str = "SHA256SUMS";
/// The minisign public key releases are signed with, base64 as printed in the
/// `.pub` file. While it is empty, releases aren't signed and the download is
/// only checked against SHA256SUMS, which proves nothing about who built it.
/// Fill it in with the line after `untrusted comment:` in `~/.minisign/minisign.pub`
/// to turn signing on; see "Signing releases" in README.md.
const PUBLIC_KEY: &str = "RWQKf1RWL1og9LBKjOF0YnSYW28sXfp4B1wCTZ76/A1Z6sQCiESxB4Aw";
const SIGNATURE_SUFFIX: &str = ".minisig";
/// Caps on what a response may cost us, so a hostile or broken server can't
/// exhaust memory or fill the disk.
const MAX_JSON: u64 = 1024 * 1024;
const MAX_SUMS: u64 = 64 * 1024;
const MAX_SIGNATURE: u64 = 8 * 1024;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
/// How many releases to ask GitHub for. The newest is always in the first
/// page, and a page is enough to find the highest version either channel wants.
const RELEASES_PER_PAGE: usize = 30;

/// Which releases the user is willing to be offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    /// Only finished releases.
    #[default]
    Stable,
    /// Finished releases and pre-releases.
    Beta,
}

impl Channel {
    pub fn from_code(code: &str) -> Channel {
        match code {
            "beta" => Channel::Beta,
            _ => Channel::Stable,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }

    /// Whether a release GitHub marks as a pre-release may be offered.
    fn accepts(self, prerelease: bool) -> bool {
        !prerelease || self == Channel::Beta
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    UpToDate,
    Available(Release),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub assets: Vec<Asset>,
    /// GitHub's `prerelease` flag, so the banner can say what is being offered.
    pub prerelease: bool,
}

/// A file attached to a GitHub release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// Checks in the background and hands the result to `done`.
pub fn spawn_check(channel: Channel, done: impl FnOnce(UpdateStatus) + Send + 'static) {
    std::thread::spawn(move || done(check(channel).unwrap_or_else(UpdateStatus::Failed)));
}

fn check(channel: Channel) -> Result<UpdateStatus, String> {
    let repo = REPOSITORY
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")
        .filter(|r| r.split('/').count() == 2)
        .ok_or("No update source is configured")?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    // The list rather than /releases/latest: the beta channel needs the
    // pre-releases, which /latest leaves out, and picking the highest version
    // beats trusting whichever release was published last.
    let mut response = agent
        .get(format!(
            "https://api.github.com/repos/{repo}/releases?per_page={RELEASES_PER_PAGE}"
        ))
        .header("User-Agent", format!("ether-ovpn/{CURRENT}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    match response.status().as_u16() {
        200 => {}
        404 => return Err("No releases have been published yet".to_owned()),
        code => return Err(format!("GitHub answered with HTTP {code}")),
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_JSON)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let releases: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let list = releases
        .as_array()
        .ok_or("GitHub sent something other than a list of releases")?;
    if list.is_empty() {
        return Err("No releases have been published yet".to_owned());
    }

    Ok(match newest_release(list, channel) {
        Some(release) if is_newer(&release.version, CURRENT) => UpdateStatus::Available(release),
        _ => UpdateStatus::UpToDate,
    })
}

/// The highest version in `releases` that `channel` accepts.
///
/// A release whose tag isn't a version is skipped rather than sorted to the
/// bottom, so a stray tag can't hide a real update.
fn newest_release(releases: &[serde_json::Value], channel: Channel) -> Option<Release> {
    releases
        .iter()
        .filter_map(parse_release)
        .filter(|release| channel.accepts(release.prerelease))
        .filter_map(|release| Some((Version::parse(&release.version)?, release)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, release)| release)
}

fn parse_release(value: &serde_json::Value) -> Option<Release> {
    let tag = value["tag_name"].as_str()?;
    let assets = value["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Asset {
                name: a["name"].as_str()?.to_owned(),
                url: a["browser_download_url"].as_str()?.to_owned(),
                size: a["size"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        url: value["html_url"].as_str().unwrap_or(REPOSITORY).to_owned(),
        assets,
        prerelease: value["prerelease"].as_bool().unwrap_or(false),
    })
}

/// A version as `major.minor.patch` plus an optional pre-release suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    core: (u64, u64, u64),
    /// Empty for a finished release, `["beta", "1"]` for `0.4.0-beta.1`.
    pre: Vec<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Version> {
        // Build metadata is ignored, as semver says.
        let text = text.trim().trim_start_matches('v').split('+').next()?;
        let (core, pre) = text.split_once('-').unwrap_or((text, ""));
        let mut parts = core.split('.').map(|p| p.parse::<u64>());
        let core = (
            parts.next()?.ok()?,
            parts.next().unwrap_or(Ok(0)).ok()?,
            parts.next().unwrap_or(Ok(0)).ok()?,
        );
        let pre = pre.split('.').filter(|s| !s.is_empty()).map(str::to_owned).collect();
        Some(Version { core, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                // A finished release outranks any pre-release of the same version.
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => compare_prerelease(&self.pre, &other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Orders pre-release identifiers the way semver does: numeric ones compare as
/// numbers and sort below alphanumeric ones, and a shorter list that is a
/// prefix of a longer one is the older version.
fn compare_prerelease(a: &[String], b: &[String]) -> Ordering {
    for (left, right) in a.iter().zip(b) {
        let order = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => left.cmp(right),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

/// Whether `candidate` is a later version than `current`.
fn is_newer(candidate: &str, current: &str) -> bool {
    matches!(
        (Version::parse(candidate), Version::parse(current)),
        (Some(a), Some(b)) if a > b
    )
}

pub fn current_version() -> &'static str {
    CURRENT
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    // The URL comes from the API response, and xdg-open would read a leading
    // dash as an option, so only a GitHub page is ever handed over.
    if !url.starts_with("https://github.com/") {
        return;
    }
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// How this copy of Ether was installed, which decides how to update it.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// A standalone binary that we replace in place (the release tarball).
    Binary(PathBuf),
    /// The .deb package in /usr.
    Deb,
    /// The .rpm package in /usr.
    Rpm,
    /// The Flatpak, installed per user or system-wide.
    Flatpak { app_id: String, user: bool },
}

impl Install {
    pub fn detect() -> Result<Self, String> {
        if std::env::consts::ARCH != "x86_64" {
            return Err("Updates are only published for x86_64".to_owned());
        }
        if let Ok(app_id) = std::env::var("FLATPAK_ID") {
            // System-wide Flatpaks are deployed under /var/lib/flatpak.
            let info = fs::read_to_string("/.flatpak-info").unwrap_or_default();
            let user = !info.lines().any(|l| l.starts_with("app-path=/var/lib/flatpak/"));
            return Ok(Self::Flatpak { app_id, user });
        }
        let exe = std::env::current_exe().map_err(|e| format!("Can't find the Ether program: {e}"))?;
        if exe.components().any(|c| c.as_os_str() == "target") {
            return Err("This copy was built from source. Update it with git pull and cargo build.".to_owned());
        }
        if exe.starts_with("/usr/") {
            let from_rpm = Command::new("rpm")
                .arg("-qf")
                .arg(&exe)
                .output()
                .is_ok_and(|o| o.status.success());
            return Ok(if from_rpm { Self::Rpm } else { Self::Deb });
        }
        Ok(Self::Binary(exe))
    }

    /// The release file to download for this kind of install.
    fn asset_suffix(&self) -> &'static str {
        match self {
            Self::Binary(_) => "-x86_64-linux.tar.gz",
            Self::Deb => "_amd64.deb",
            Self::Rpm => ".x86_64.rpm",
            Self::Flatpak { .. } => "-x86_64.flatpak",
        }
    }

    /// Whether installing will ask for the admin password.
    pub fn asks_for_password(&self) -> bool {
        matches!(self, Self::Deb | Self::Rpm) || matches!(self, Self::Flatpak { user: false, .. })
    }

    /// Starts the updated copy of Ether. The caller then closes this one.
    pub fn relaunch(&self) -> Result<(), String> {
        let result = match self {
            Self::Binary(exe) => Command::new(exe).spawn(),
            Self::Deb | Self::Rpm => Command::new("/usr/bin/ether-ovpn").spawn(),
            Self::Flatpak { app_id, .. } => Command::new("flatpak-spawn")
                .args(["--host", "flatpak", "run", app_id])
                .spawn(),
        };
        result.map(drop).map_err(|e| format!("Couldn't restart Ether: {e}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Fraction of the download done.
    Downloading(f32),
    Installing,
    Done,
    Failed(String),
}

/// Downloads, verifies and installs the update in the background, reporting
/// progress through `report`.
pub fn spawn_install(install: Install, assets: Vec<Asset>, report: impl Fn(Progress) + Send + 'static) {
    std::thread::spawn(move || match download_and_install(&install, &assets, &report) {
        Ok(()) => report(Progress::Done),
        Err(e) => report(Progress::Failed(e)),
    });
}

fn download_and_install(install: &Install, assets: &[Asset], report: &dyn Fn(Progress)) -> Result<(), String> {
    let suffix = install.asset_suffix();
    let asset = assets
        .iter()
        .find(|a| a.name.ends_with(suffix))
        .ok_or_else(|| format!("The release has no {suffix} file"))?;
    let sums = assets
        .iter()
        .find(|a| a.name == CHECKSUMS)
        .ok_or("The release has no checksums to verify the download with")?;

    let dir = dirs::cache_dir()
        .ok_or("No cache folder to download to")?
        .join("ether")
        .join("updates");
    fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let file = dir.join(&asset.name);

    report(Progress::Downloading(0.0));
    let result = (|| {
        let digest = download(asset, &file, report)?;
        let expected = expected_checksum(sums, &asset.name)?;
        if digest != expected {
            return Err("The download is damaged (its checksum doesn't match). Try again.".to_owned());
        }
        verify_signature(assets, &file, &asset.name)?;
        report(Progress::Installing);
        match install {
            Install::Binary(exe) => replace_binary(&file, exe),
            Install::Deb => run_installer(
                Command::new("pkexec")
                    .args(["apt-get", "install", "-y", "--allow-downgrades"])
                    .arg(&file),
            ),
            // The .rpm isn't signed, which zypper refuses without being told.
            Install::Rpm if Path::new("/usr/bin/dnf").exists() => {
                run_installer(Command::new("pkexec").args(["dnf", "install", "-y"]).arg(&file))
            }
            Install::Rpm => run_installer(
                Command::new("pkexec")
                    .args([
                        "zypper",
                        "--non-interactive",
                        "install",
                        "--allow-unsigned-rpm",
                        "--oldpackage",
                    ])
                    .arg(&file),
            ),
            Install::Flatpak { user, .. } => run_installer(
                Command::new("flatpak-spawn")
                    .args(["--host", "flatpak", "install", "--reinstall", "-y", "--noninteractive"])
                    .arg(if *user { "--user" } else { "--system" })
                    .arg(&file),
            ),
        }
    })();
    let _ = fs::remove_file(&file);
    result
}

fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

/// Downloads to `path`, returning the SHA-256 of the file as lowercase hex.
fn download(asset: &Asset, path: &Path, report: &dyn Fn(Progress)) -> Result<String, String> {
    let mut response = download_agent()
        .get(&asset.url)
        .header("User-Agent", format!("ether-ovpn/{CURRENT}"))
        .call()
        .map_err(|e| format!("Couldn't download the update: {e}"))?;
    let total = response.body().content_length().unwrap_or(asset.size).max(1);
    let mut reader = response.body_mut().as_reader();
    let mut out = File::create(path).map_err(|e| format!("Can't save the download: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let (mut done, mut last_report) = (0u64, 0.0f32);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("The download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| format!("Can't save the download: {e}"))?;
        done += n as u64;
        if done > MAX_DOWNLOAD {
            return Err("The download is far larger than a release should be".to_owned());
        }
        let fraction = (done as f32 / total as f32).min(1.0);
        if fraction - last_report >= 0.01 {
            last_report = fraction;
            report(Progress::Downloading(fraction));
        }
    }
    out.flush().map_err(|e| format!("Can't save the download: {e}"))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Looks up `name` in the release's SHA256SUMS file.
fn expected_checksum(sums: &Asset, name: &str) -> Result<String, String> {
    let mut response = download_agent()
        .get(&sums.url)
        .header("User-Agent", format!("ether-ovpn/{CURRENT}"))
        .call()
        .map_err(|e| format!("Couldn't download the checksums: {e}"))?;
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_SUMS)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    checksum_for(&text, name).ok_or_else(|| format!("{name} isn't listed in {CHECKSUMS}"))
}

fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

/// Checks the archive against the release's minisign signature.
///
/// The checksum only proves the download matches the release page; the
/// signature proves the release page is ours. With no key configured there is
/// nothing to check against, so the step is skipped.
fn verify_signature(assets: &[Asset], file: &Path, name: &str) -> Result<(), String> {
    verify_signature_with_key(PUBLIC_KEY, assets, file, name)
}

/// The same with the key passed in, so the empty-key skip can be tested
/// without depending on what this build has configured.
fn verify_signature_with_key(key: &str, assets: &[Asset], file: &Path, name: &str) -> Result<(), String> {
    if key.is_empty() {
        return Ok(());
    }
    let asset = signature_asset(assets, name)?;
    let text = fetch_signature(asset)?;
    let bytes = fs::read(file).map_err(|e| format!("Can't read the download back: {e}"))?;
    verify_with_key(key, &bytes, &text)
}

/// Finds the `.minisig` file that belongs to `name` in the release.
fn signature_asset<'a>(assets: &'a [Asset], name: &str) -> Result<&'a Asset, String> {
    let signature_name = format!("{name}{SIGNATURE_SUFFIX}");
    assets.iter().find(|a| a.name == signature_name).ok_or_else(|| {
        format!("This release has no {signature_name}, so the update can't be verified and won't be installed")
    })
}

fn fetch_signature(asset: &Asset) -> Result<String, String> {
    let mut response = download_agent()
        .get(&asset.url)
        .header("User-Agent", format!("ether-ovpn/{CURRENT}"))
        .call()
        .map_err(|e| format!("Couldn't download the signature: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_SIGNATURE)
        .read_to_string()
        .map_err(|e| format!("Couldn't read the signature: {e}"))
}

/// Checks `data` against a minisign signature made with `public_key`.
fn verify_with_key(public_key: &str, data: &[u8], signature_text: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(public_key)
        .map_err(|e| format!("The built-in release key is broken: {e}"))?;
    let signature = minisign_verify::Signature::decode(signature_text)
        .map_err(|e| format!("The update's signature is unreadable: {e}"))?;
    key.verify(data, &signature, false).map_err(|e| match e {
        minisign_verify::Error::UnexpectedKeyId => {
            "The update was signed with a key that isn't Ether's, so it wasn't installed".to_owned()
        }
        minisign_verify::Error::InvalidSignature => {
            "The update's signature doesn't match the download, so it wasn't installed".to_owned()
        }
        minisign_verify::Error::UnexpectedAlgorithm | minisign_verify::Error::UnsupportedAlgorithm => {
            "The update's signature is in an old format Ether can't check, so it wasn't installed".to_owned()
        }
        other => format!("The update's signature couldn't be checked: {other}"),
    })
}

/// Swaps in the `ether-ovpn` binary from a release tarball. The running
/// program keeps working: renaming over it doesn't touch the open file.
fn replace_binary(tarball: &Path, exe: &Path) -> Result<(), String> {
    let dir = exe.parent().ok_or("Can't find the folder Ether is in")?;
    let staged = dir.join(".ether-ovpn-update");
    let file = File::open(tarball).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file)));
    let mut found = false;
    for entry in archive
        .entries()
        .map_err(|e| format!("The download isn't a valid archive: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("The download isn't a valid archive: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        // The binary sits at ether-ovpn-<version>-x86_64-linux/ether-ovpn.
        if path.components().count() == 2 && path.file_name().is_some_and(|n| n == "ether-ovpn") {
            let mut out = File::create(&staged)
                .map_err(|e| format!("Can't write to {} ({e}). Install the update by hand.", dir.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            found = true;
            break;
        }
    }
    if !found {
        return Err("The archive doesn't contain the Ether program".to_owned());
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    fs::rename(&staged, exe).map_err(|e| {
        let _ = fs::remove_file(&staged);
        format!("Can't replace {}: {e}", exe.display())
    })
}

/// Runs a package manager, turning its exit status and output into an error.
fn run_installer(command: &mut Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{program} isn't available, so the update can't be installed here"),
        _ => format!("Couldn't run {program}: {e}"),
    })?;
    if output.status.success() {
        return Ok(());
    }
    // pkexec exits with 126 when the password prompt is dismissed.
    if program == "pkexec" && matches!(output.status.code(), Some(126 | 127)) {
        return Err("The installation was cancelled".to_owned());
    }
    let last_line = BufReader::new(&output.stderr[..])
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or_else(|| format!("{program} failed ({})", output.status));
    Err(last_line)
}

#[cfg(test)]
mod tests {
    use super::{
        Asset, Channel, PUBLIC_KEY, Version, checksum_for, is_newer, newest_release, parse_release, signature_asset,
        verify_signature_with_key, verify_with_key,
    };
    use std::path::Path;

    const TEST_KEY: &str = include_str!("../tests/fixtures/minisign/test-key.pub");
    const OTHER_KEY: &str = include_str!("../tests/fixtures/minisign/other-key.pub");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/minisign/payload.txt");
    const TAMPERED: &[u8] = include_bytes!("../tests/fixtures/minisign/payload-tampered.txt");
    const SIGNATURE: &str = include_str!("../tests/fixtures/minisign/payload.txt.minisig");
    const LEGACY: &str = include_str!("../tests/fixtures/minisign/payload.txt.legacy.minisig");
    const GARBAGE: &str = include_str!("../tests/fixtures/minisign/garbage.minisig");

    /// The base64 line of a `.pub` file, which is what `PUBLIC_KEY` holds.
    fn key_line(pub_file: &str) -> &str {
        pub_file.lines().nth(1).expect("the fixture has a key line")
    }

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_owned(),
            url: format!("https://example.invalid/{name}"),
            size: 0,
        }
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }

    #[test]
    fn orders_prereleases_the_way_semver_does() {
        // Without this a beta user never sees beta.2: both parse to 0.4.0.
        assert!(is_newer("0.4.0-beta.2", "0.4.0-beta.1"));
        assert!(is_newer("0.4.0-beta.10", "0.4.0-beta.9"));
        assert!(is_newer("0.4.0-rc.1", "0.4.0-beta.9"));
        assert!(is_newer("0.4.0-beta.1", "0.3.1"));
        assert!(!is_newer("0.4.0-beta.1", "0.4.0-beta.2"));
        // A shorter identifier list that is a prefix of a longer one is older.
        assert!(is_newer("0.4.0-beta.1", "0.4.0-beta"));
        // Build metadata doesn't change the order.
        assert!(!is_newer("0.4.0+build.2", "0.4.0+build.1"));
    }

    #[test]
    fn a_finished_release_outranks_its_prerelease() {
        assert!(is_newer("0.4.0", "0.4.0-rc.1"));
        assert!(!is_newer("0.4.0-rc.1", "0.4.0"));
        assert_eq!(Version::parse("0.4.0"), Version::parse("v0.4.0"));
    }

    /// A release as GitHub's `/releases` returns it, cut down to what we read.
    fn release_json(tag: &str, prerelease: bool) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/Etherhaus/ether-ovpn/releases/tag/{tag}"),
            "prerelease": prerelease,
            "assets": [{
                "name": "SHA256SUMS",
                "browser_download_url": "https://example.invalid/SHA256SUMS",
                "size": 12,
            }],
        })
    }

    #[test]
    fn offers_a_prerelease_only_on_the_beta_channel() {
        let releases = [release_json("v0.4.0-beta.1", true), release_json("v0.3.1", false)];

        let stable = newest_release(&releases, Channel::Stable).expect("a stable release");
        assert_eq!(stable.version, "0.3.1");
        assert!(!stable.prerelease);

        let beta = newest_release(&releases, Channel::Beta).expect("a beta release");
        assert_eq!(beta.version, "0.4.0-beta.1");
        assert!(beta.prerelease);
    }

    #[test]
    fn picks_the_highest_version_not_the_newest_published() {
        // GitHub lists releases newest-first by publication date, so a patch to
        // an old line can sit above a newer one.
        let releases = [release_json("v0.3.2", false), release_json("v0.4.0", false)];
        assert_eq!(newest_release(&releases, Channel::Stable).unwrap().version, "0.4.0");
    }

    #[test]
    fn skips_a_tag_that_isnt_a_version() {
        let releases = [release_json("nightly", false), release_json("v0.4.0", false)];
        assert_eq!(newest_release(&releases, Channel::Stable).unwrap().version, "0.4.0");
        // Nothing usable at all is not an update, and not a crash either.
        assert_eq!(newest_release(&[release_json("nightly", false)], Channel::Beta), None);
        assert_eq!(newest_release(&[], Channel::Beta), None);
    }

    #[test]
    fn reads_a_release_out_of_the_api_response() {
        let release = parse_release(&release_json("v0.4.0-beta.1", true)).expect("a release");
        assert_eq!(release.version, "0.4.0-beta.1");
        assert!(release.prerelease);
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "SHA256SUMS");
        // A release without a tag can't be compared, so it is dropped.
        assert_eq!(parse_release(&serde_json::json!({"prerelease": false})), None);
    }

    #[test]
    fn reads_the_channel_code() {
        assert_eq!(Channel::from_code("beta"), Channel::Beta);
        assert_eq!(Channel::from_code("stable"), Channel::Stable);
        // Anything unknown is the safe choice.
        assert_eq!(Channel::from_code("nightly"), Channel::Stable);
        assert_eq!(Channel::from_code(""), Channel::Stable);
        assert_eq!(Channel::Beta.code(), "beta");
        assert_eq!(Channel::Stable.code(), "stable");
    }

    #[test]
    fn finds_checksums() {
        let sums = "ABC123  ether-ovpn_0.2.0_amd64.deb\ndef456 *ether-ovpn-0.2.0-x86_64-linux.tar.gz\n";
        assert_eq!(
            checksum_for(sums, "ether-ovpn_0.2.0_amd64.deb").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            checksum_for(sums, "ether-ovpn-0.2.0-x86_64-linux.tar.gz").as_deref(),
            Some("def456")
        );
        assert_eq!(checksum_for(sums, "other"), None);
    }

    #[test]
    fn accepts_a_signature_from_the_release_key() {
        assert_eq!(verify_with_key(key_line(TEST_KEY), PAYLOAD, SIGNATURE), Ok(()));
    }

    #[test]
    fn finds_the_signature_of_the_downloaded_file() {
        let assets = [asset("payload.txt.minisig"), asset("other.minisig")];
        assert_eq!(
            signature_asset(&assets, "payload.txt").unwrap().name,
            "payload.txt.minisig"
        );
    }

    #[test]
    fn refuses_a_release_without_a_signature() {
        let assets = [asset("payload.txt"), asset("SHA256SUMS")];
        let error = signature_asset(&assets, "payload.txt").unwrap_err();
        assert!(error.contains("payload.txt.minisig"), "{error}");
        assert!(error.contains("won't be installed"), "{error}");
    }

    #[test]
    fn refuses_a_download_that_doesnt_match_its_signature() {
        let error = verify_with_key(key_line(TEST_KEY), TAMPERED, SIGNATURE).unwrap_err();
        assert!(error.contains("doesn't match"), "{error}");
    }

    #[test]
    fn refuses_a_legacy_signature() {
        let error = verify_with_key(key_line(TEST_KEY), PAYLOAD, LEGACY).unwrap_err();
        assert!(error.contains("old format"), "{error}");
    }

    #[test]
    fn refuses_an_unreadable_signature() {
        let error = verify_with_key(key_line(TEST_KEY), PAYLOAD, GARBAGE).unwrap_err();
        assert!(error.contains("unreadable"), "{error}");
    }

    #[test]
    fn refuses_a_signature_from_another_key() {
        let error = verify_with_key(key_line(OTHER_KEY), PAYLOAD, SIGNATURE).unwrap_err();
        assert!(error.contains("isn't Ether's"), "{error}");
    }

    #[test]
    fn refuses_a_broken_built_in_key() {
        let error = verify_with_key("not a key", PAYLOAD, SIGNATURE).unwrap_err();
        assert!(error.contains("release key is broken"), "{error}");
    }

    #[test]
    fn skips_the_signature_check_while_no_key_is_configured() {
        // A build without a key has nothing to check against, so the step must
        // not block an install.
        let skipped = verify_signature_with_key("", &[], Path::new("/nonexistent/update.tar.gz"), "update.tar.gz");
        assert!(skipped.is_ok());
    }

    #[test]
    fn the_built_in_key_is_a_usable_key() {
        // A fork may ship without a key; then there is nothing to check.
        if PUBLIC_KEY.is_empty() {
            return;
        }
        // The fixture is signed by another key, so the check has to fail on the
        // mismatch, not on a key that can't be parsed.
        let error = verify_with_key(PUBLIC_KEY, PAYLOAD, SIGNATURE).unwrap_err();
        assert!(error.contains("isn't Ether's"), "{error}");
    }
}
