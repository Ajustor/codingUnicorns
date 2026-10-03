//! Self-update from GitHub releases.
//!
//! Flow: `check()` queries `releases/latest` on a background thread and compares the tag with
//! `CARGO_PKG_VERSION`. If newer, the UI offers to install. `install()` downloads the asset for
//! this platform, verifies its SHA-256 against the digest GitHub publishes, then either:
//! - replaces the running executable in place (portable binaries), or
//! - on Windows MSI installs (Program Files, not writable), stages the `.msi` so it can be run
//!   with `msiexec` once the app has exited.
//!
//! Network and disk work never runs on the UI thread; results come back over a channel polled
//! once per frame by `poll()`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const REPO: &str = "Ajustor/codingUnicorns";
const USER_AGENT: &str = concat!("coding-unicorns/", env!("CARGO_PKG_VERSION"));
/// Hard cap on downloaded asset size, as a guard against a runaway response.
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const BINARY_ASSET: Option<&str> = Some("coding-unicorns-windows-x64.exe");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BINARY_ASSET: Option<&str> = Some("coding-unicorns-linux-x64");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const BINARY_ASSET: Option<&str> = Some("coding-unicorns-macos-arm64");
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
)))]
const BINARY_ASSET: Option<&str> = None;

const MSI_ASSET: &str = "coding-unicorns-setup.msi";

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    /// `"sha256:<hex>"`, published by GitHub for every uploaded asset.
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: semver::Version,
    pub notes: String,
    pub html_url: String,
    asset: GhAsset,
    kind: InstallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallKind {
    /// Overwrite the running executable.
    ReplaceBinary,
    /// Run the MSI installer after exit (Windows, installed under Program Files).
    Msi,
}

#[derive(Debug, Clone)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(ReleaseInfo),
    Downloading(ReleaseInfo),
    /// The update is installed (binary) or staged (MSI); a restart applies it.
    Ready(ReleaseInfo),
    Failed(String),
}

/// What the app should do on exit to apply a ready update.
#[derive(Debug, Clone)]
pub enum ExitAction {
    Relaunch,
    RunMsi(PathBuf),
}

enum Msg {
    Checked(Result<Option<ReleaseInfo>, String>),
    Installed(Result<Option<PathBuf>, String>),
}

pub struct Updater {
    pub state: UpdateState,
    /// The "update available" dialog was dismissed for this session.
    pub dismissed: bool,
    /// True when the current check was started by the user (show "up to date" / errors).
    manual: bool,
    staged_msi: Option<PathBuf>,
    pub exit_action: Option<ExitAction>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

/// Event surfaced to the app after `poll()`, typically turned into a toast.
pub enum UpdateEvent {
    UpToDate,
    Available(semver::Version),
    Ready,
    Error(String),
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            state: UpdateState::Idle,
            dismissed: false,
            manual: false,
            staged_msi: None,
            exit_action: None,
            tx,
            rx,
        }
    }

    pub fn current_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.state,
            UpdateState::Checking | UpdateState::Downloading(_)
        )
    }

    /// Start a background check. `manual` checks report "up to date" and errors to the user,
    /// and ignore `skipped_version`.
    pub fn check(&mut self, manual: bool, skipped_version: Option<&str>, ctx: &egui::Context) {
        if self.is_busy() || matches!(self.state, UpdateState::Ready(_)) {
            return;
        }
        self.manual = manual;
        self.dismissed = false;
        self.state = UpdateState::Checking;
        let skipped = if manual {
            None
        } else {
            skipped_version.and_then(|v| semver::Version::parse(v).ok())
        };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res =
                fetch_latest().map(|r| r.filter(|info| Some(&info.version) != skipped.as_ref()));
            let _ = tx.send(Msg::Checked(res));
            ctx.request_repaint();
        });
    }

    /// Download and install (or stage) the available release.
    pub fn install(&mut self, ctx: &egui::Context) {
        let UpdateState::Available(info) = &self.state else {
            return;
        };
        let info = info.clone();
        self.state = UpdateState::Downloading(info.clone());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = download_and_apply(&info);
            let _ = tx.send(Msg::Installed(res));
            ctx.request_repaint();
        });
    }

    /// Arrange for the update to be applied when the app exits.
    pub fn schedule_restart(&mut self) {
        if !matches!(self.state, UpdateState::Ready(_)) {
            return;
        }
        self.exit_action = Some(match &self.staged_msi {
            Some(msi) => ExitAction::RunMsi(msi.clone()),
            None => ExitAction::Relaunch,
        });
    }

    /// Apply a ready update when the app quits, without relaunching. Only the MSI needs
    /// this: a replaced binary is picked up on the next launch by itself.
    pub fn schedule_on_quit(&mut self) {
        if let (UpdateState::Ready(_), Some(msi)) = (&self.state, &self.staged_msi) {
            self.exit_action = Some(ExitAction::RunMsi(msi.clone()));
        }
    }

    /// Drain worker results. Call once per frame.
    pub fn poll(&mut self) -> Option<UpdateEvent> {
        let msg = self.rx.try_recv().ok()?;
        match msg {
            Msg::Checked(Ok(Some(info))) => {
                let v = info.version.clone();
                self.state = UpdateState::Available(info);
                Some(UpdateEvent::Available(v))
            }
            Msg::Checked(Ok(None)) => {
                self.state = UpdateState::UpToDate;
                self.manual.then_some(UpdateEvent::UpToDate)
            }
            Msg::Checked(Err(e)) => {
                log::warn!("update check failed: {e}");
                self.state = UpdateState::Failed(e.clone());
                self.manual.then_some(UpdateEvent::Error(e))
            }
            Msg::Installed(res) => {
                let UpdateState::Downloading(info) =
                    std::mem::replace(&mut self.state, UpdateState::Idle)
                else {
                    return None;
                };
                match res {
                    Ok(staged) => {
                        self.staged_msi = staged;
                        self.state = UpdateState::Ready(info);
                        Some(UpdateEvent::Ready)
                    }
                    Err(e) => {
                        log::warn!("update install failed: {e}");
                        self.state = UpdateState::Failed(e.clone());
                        // Installs are always user-initiated, so always report.
                        Some(UpdateEvent::Error(e))
                    }
                }
            }
        }
    }
}

/// Apply `action` as the app shuts down. `workspace` is reopened on relaunch.
pub fn run_exit_action(action: &ExitAction, workspace: Option<&std::path::Path>) {
    let result = match action {
        ExitAction::Relaunch => std::env::current_exe().and_then(|exe| {
            let mut cmd = std::process::Command::new(exe);
            if let Some(ws) = workspace {
                cmd.arg(ws);
            }
            cmd.spawn().map(|_| ())
        }),
        // `/passive` shows a progress bar and triggers the UAC prompt; MajorUpgrade in the
        // WiX manifest removes the previous version.
        ExitAction::RunMsi(msi) => std::process::Command::new("msiexec")
            .arg("/i")
            .arg(msi)
            .arg("/passive")
            .spawn()
            .map(|_| ()),
    };
    if let Err(e) = result {
        log::error!("failed to apply update on exit: {e}");
    }
}

fn http_get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("request to {url} failed: {e}"))
}

fn fetch_latest() -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = http_get(&url)?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("reading release info: {e}"))?;
    let release: GhRelease =
        serde_json::from_str(&text).map_err(|e| format!("parsing release info: {e}"))?;
    let current = semver::Version::parse(Updater::current_version())
        .map_err(|e| format!("bad current version: {e}"))?;
    select_update(release, &current, install_kind())
}

/// Pick the asset to install if `release` is newer than `current`.
fn select_update(
    release: GhRelease,
    current: &semver::Version,
    kind: InstallKind,
) -> Result<Option<ReleaseInfo>, String> {
    let version = semver::Version::parse(release.tag_name.trim_start_matches('v'))
        .map_err(|e| format!("bad release tag {:?}: {e}", release.tag_name))?;
    if version <= *current {
        return Ok(None);
    }
    let wanted = match kind {
        InstallKind::Msi => MSI_ASSET,
        InstallKind::ReplaceBinary => BINARY_ASSET.ok_or("no prebuilt binary for this platform")?,
    };
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == wanted)
        .cloned()
        .ok_or_else(|| format!("release {} has no asset {wanted}", release.tag_name))?;
    Ok(Some(ReleaseInfo {
        version,
        notes: release.body.unwrap_or_default(),
        html_url: release.html_url,
        asset,
        kind,
    }))
}

/// MSI installs live under Program Files, which a normal user can't write to, so the
/// installer must handle the upgrade. Everything else is a portable binary we replace.
fn install_kind() -> InstallKind {
    if cfg!(windows) {
        let in_program_files = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                let pf = std::env::var_os("ProgramFiles")?;
                Some(exe.starts_with(pf))
            })
            .unwrap_or(false);
        if in_program_files {
            return InstallKind::Msi;
        }
    }
    InstallKind::ReplaceBinary
}

/// Returns the staged MSI path for MSI installs, `None` once the binary has been replaced.
fn download_and_apply(info: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
    let mut resp = http_get(&info.asset.browser_download_url)?;
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_ASSET_BYTES)
        .read_to_vec()
        .map_err(|e| format!("downloading {}: {e}", info.asset.name))?;
    verify_digest(&bytes, info.asset.digest.as_deref())?;

    let dir = std::env::temp_dir().join("coding-unicorns-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(&info.asset.name);
    std::fs::write(&path, &bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;

    match info.kind {
        InstallKind::Msi => Ok(Some(path)),
        InstallKind::ReplaceBinary => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("chmod {}: {e}", path.display()))?;
            }
            let res =
                self_replace::self_replace(&path).map_err(|e| format!("replacing executable: {e}"));
            let _ = std::fs::remove_file(&path);
            res.map(|_| None)
        }
    }
}

/// Refuse to install anything whose hash we can't check against GitHub's digest.
fn verify_digest(bytes: &[u8], digest: Option<&str>) -> Result<(), String> {
    let expected = digest
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or("release asset has no sha256 digest; refusing to install")?;
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch (expected {expected}, got {actual})"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            html_url: String::new(),
            body: None,
            assets: assets
                .iter()
                .map(|n| GhAsset {
                    name: (*n).into(),
                    browser_download_url: String::new(),
                    digest: None,
                })
                .collect(),
        }
    }

    #[test]
    fn older_or_equal_release_is_not_an_update() {
        let cur = semver::Version::new(0, 5, 0);
        let r = release("v0.5.0", &[MSI_ASSET]);
        assert!(select_update(r, &cur, InstallKind::Msi).unwrap().is_none());
        let r = release("v0.4.9", &[MSI_ASSET]);
        assert!(select_update(r, &cur, InstallKind::Msi).unwrap().is_none());
    }

    #[test]
    fn newer_release_picks_matching_asset() {
        let cur = semver::Version::new(0, 5, 0);
        let r = release("v0.6.0", &["other", MSI_ASSET]);
        let info = select_update(r, &cur, InstallKind::Msi).unwrap().unwrap();
        assert_eq!(info.version, semver::Version::new(0, 6, 0));
        assert_eq!(info.asset.name, MSI_ASSET);
    }

    #[test]
    fn newer_release_without_asset_errors() {
        let cur = semver::Version::new(0, 5, 0);
        let r = release("v0.6.0", &["other"]);
        assert!(select_update(r, &cur, InstallKind::Msi).is_err());
    }

    #[test]
    fn digest_verification() {
        // sha256("abc")
        let abc = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_digest(b"abc", Some(abc)).is_ok());
        assert!(verify_digest(b"abd", Some(abc)).is_err());
        assert!(verify_digest(b"abc", None).is_err());
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// Hits the real GitHub API: `cargo test -- --ignored live_release`.
    #[test]
    #[ignore]
    fn live_release_has_installable_assets() {
        let mut resp = http_get(&format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .unwrap();
        let release: GhRelease =
            serde_json::from_str(&resp.body_mut().read_to_string().unwrap()).unwrap();
        let old = semver::Version::new(0, 0, 1);
        for kind in [InstallKind::ReplaceBinary, InstallKind::Msi] {
            let info = select_update(release.clone(), &old, kind).unwrap().unwrap();
            assert!(info.asset.digest.as_deref().unwrap().starts_with("sha256:"));
        }
    }
}
