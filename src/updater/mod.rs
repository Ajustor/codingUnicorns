//! Self-update from the release manifest published on GitHub Pages.
//!
//! The repository is private, so clients can't use the GitHub releases API. Instead the release
//! workflow deploys `latest.json` plus the binaries to GitHub Pages (public).
//!
//! Flow: `check()` fetches the manifest on a background thread and compares its version with
//! `CARGO_PKG_VERSION`. If newer, the UI offers to install. `install()` downloads the asset for
//! this platform, verifies its SHA-256 against the manifest, then either:
//! - replaces the running executable in place (portable binaries, the macOS `.app` bundle), or
//! - replaces the `.AppImage` file the app runs from (Linux AppImage), or
//! - on Windows MSI installs (Program Files, not writable), stages the `.msi` so it can be run
//!   with `msiexec` once the app has exited.
//!
//! Network and disk work never runs on the UI thread; results come back over a channel polled
//! once per frame by `poll()`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Written by the `pages` job of `.github/workflows/release.yml`.
const MANIFEST_URL: &str = "https://ajustor.github.io/codingUnicorns/latest.json";
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const APPIMAGE_ASSET: Option<&str> = Some("coding-unicorns-linux-x64.AppImage");
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
const APPIMAGE_ASSET: Option<&str> = None;

/// `latest.json` on GitHub Pages.
#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    /// Human-facing download page.
    page_url: String,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestAsset {
    name: String,
    url: String,
    /// Lowercase hex SHA-256 of the file.
    sha256: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: semver::Version,
    pub notes: String,
    pub page_url: String,
    asset: ManifestAsset,
    kind: InstallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallKind {
    /// Overwrite the running executable.
    ReplaceBinary,
    /// Run the MSI installer after exit (Windows, installed under Program Files).
    Msi,
    /// Overwrite the `.AppImage` file the app was started from (Linux).
    AppImage,
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
    /// Install the MSI; the app stays closed (plain quit).
    RunMsi(PathBuf),
    /// Install the MSI, then start the upgraded app ("Restart now").
    RunMsiThenRelaunch(PathBuf),
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
        self.check_with(manual, skipped_version, ctx, fetch_latest);
    }

    /// `check()` with the manifest fetch injected (tests substitute a fake).
    fn check_with(
        &mut self,
        manual: bool,
        skipped_version: Option<&str>,
        ctx: &egui::Context,
        fetch: fn() -> Result<Option<ReleaseInfo>, String>,
    ) {
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
            let res = fetch().map(|r| r.filter(|info| Some(&info.version) != skipped.as_ref()));
            let _ = tx.send(Msg::Checked(res));
            ctx.request_repaint();
        });
    }

    /// Download and install (or stage) the available release.
    pub fn install(&mut self, ctx: &egui::Context) {
        self.install_with(ctx, download_and_apply);
    }

    /// `install()` with the download/apply step injected (tests substitute a fake).
    fn install_with(
        &mut self,
        ctx: &egui::Context,
        apply: fn(&ReleaseInfo) -> Result<Option<PathBuf>, String>,
    ) {
        let UpdateState::Available(info) = &self.state else {
            return;
        };
        let info = info.clone();
        self.state = UpdateState::Downloading(info.clone());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = apply(&info);
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
            Some(msi) => ExitAction::RunMsiThenRelaunch(msi.clone()),
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
                        // A downloaded MSI is installed when the app quits, however
                        // it is closed: before, only "Restart now" / "When I quit"
                        // scheduled it, and closing the window left it unused.
                        self.schedule_on_quit();
                        log_step(&format!("ready, exit action: {:?}", self.exit_action));
                        Some(UpdateEvent::Ready)
                    }
                    Err(e) => {
                        log::warn!("update install failed: {e}");
                        log_step(&format!("download failed: {e}"));
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
    log_step(&format!("on exit: {action:?}"));
    let result = match action {
        ExitAction::Relaunch => relaunch_exe().and_then(|exe| {
            let mut cmd = std::process::Command::new(exe);
            // We may still be listening while exiting: never forward to ourselves.
            cmd.arg(crate::single_instance::NEW_WINDOW_FLAG);
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
            .arg("/l*v")
            .arg(msi_log_path(msi))
            .spawn()
            .map(|_| ()),
        // The app is exiting, so a detached PowerShell waits for msiexec and then starts
        // the upgraded executable (MajorUpgrade keeps the install location).
        ExitAction::RunMsiThenRelaunch(msi) => std::env::current_exe().and_then(|exe| {
            let script = msi_relaunch_script(msi, &exe, workspace);
            use crate::process_ext::CommandExt as _;
            std::process::Command::new("powershell.exe")
                .no_window()
                .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden"])
                .arg("-EncodedCommand")
                .arg(encode_powershell_command(&script))
                .spawn()
                .map(|_| ())
        }),
    };
    match result {
        Ok(()) => log_step("on exit: started"),
        Err(e) => {
            log::error!("failed to apply update on exit: {e}");
            log_step(&format!("on exit: failed to start: {e}"));
        }
    }
}

/// What to start to relaunch the app: the AppImage file when running from one (the
/// executable itself lives in the image's mount, gone once the app exits).
fn relaunch_exe() -> std::io::Result<PathBuf> {
    match running_appimage() {
        Some(appimage) => Ok(appimage),
        None => std::env::current_exe(),
    }
}

/// Folder the update is downloaded to (with `update.log` and the
/// installer's `install.log`).
fn update_dir() -> PathBuf {
    std::env::temp_dir().join("coding-unicorns-update")
}

/// msiexec's verbose log, next to the staged MSI.
fn msi_log_path(msi: &std::path::Path) -> PathBuf {
    msi.with_file_name("install.log")
}

/// Append a step of the update to `update.log`: the app has no console on
/// Windows, so this is what tells why an update did not apply.
fn log_step(msg: &str) {
    use std::io::Write as _;
    if cfg!(test) {
        return; // keep the user's log free of test runs
    }
    let dir = update_dir();
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("update.log"))
    {
        let _ = writeln!(f, "[{secs}] v{} {msg}", Updater::current_version());
    }
}

/// PowerShell that installs `msi` and, on success, relaunches `exe` (with `workspace`).
fn msi_relaunch_script(
    msi: &std::path::Path,
    exe: &std::path::Path,
    workspace: Option<&std::path::Path>,
) -> String {
    // Single-quoted PowerShell literal (a `'` is doubled inside it).
    let literal =
        |p: &std::path::Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
    // `-ArgumentList` entries are joined with spaces, so paths need inner double quotes
    // (Windows paths can't contain `"`).
    let arg =
        |p: &std::path::Path| format!("'\"{}\"'", p.display().to_string().replace('\'', "''"));
    // Like `ExitAction::Relaunch`: open a window of our own, even if another
    // instance is running and listening for paths.
    let new_window = crate::single_instance::NEW_WINDOW_FLAG;
    let relaunch_args = match workspace {
        Some(ws) => format!(" -ArgumentList '{new_window}',{}", arg(ws)),
        None => format!(" -ArgumentList '{new_window}'"),
    };
    format!(
        "$p = Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i',{msi},'/passive','/l*v',{log} -Wait -PassThru\n\
         # 3010: success, reboot required\n\
         if ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) {{ Start-Process -FilePath {exe}{relaunch_args} }}\n",
        msi = arg(msi),
        log = arg(&msi_log_path(msi)),
        exe = literal(exe),
    )
}

/// Encode `script` for `powershell -EncodedCommand` (base64 of UTF-16LE).
fn encode_powershell_command(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn http_get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(crate::extension::remote_registry::tls_config())
        .build()
        .into();
    agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("request to {url} failed: {e}"))
}

fn fetch_latest() -> Result<Option<ReleaseInfo>, String> {
    let mut resp = http_get(MANIFEST_URL)?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("reading release manifest: {e}"))?;
    release_from_manifest(&text, install_kind())
}

/// Parse `latest.json` and pick the update for this build, if any.
fn release_from_manifest(text: &str, kind: InstallKind) -> Result<Option<ReleaseInfo>, String> {
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| format!("parsing release manifest: {e}"))?;
    let current = semver::Version::parse(Updater::current_version())
        .map_err(|e| format!("bad current version: {e}"))?;
    select_update(manifest, &current, kind)
}

/// Pick the asset to install if `manifest` is newer than `current`.
fn select_update(
    manifest: Manifest,
    current: &semver::Version,
    kind: InstallKind,
) -> Result<Option<ReleaseInfo>, String> {
    let version = semver::Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|e| format!("bad release version {:?}: {e}", manifest.version))?;
    if version <= *current {
        return Ok(None);
    }
    let wanted = match kind {
        InstallKind::Msi => MSI_ASSET,
        InstallKind::ReplaceBinary => BINARY_ASSET.ok_or("no prebuilt binary for this platform")?,
        InstallKind::AppImage => APPIMAGE_ASSET.ok_or("no AppImage for this platform")?,
    };
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.name == wanted)
        .cloned()
        .ok_or_else(|| format!("release v{version} has no asset {wanted}"))?;
    Ok(Some(ReleaseInfo {
        version,
        notes: manifest.notes,
        page_url: manifest.page_url,
        asset,
        kind,
    }))
}

/// MSI installs live under Program Files, which a normal user can't write to, so the
/// installer must handle the upgrade. An AppImage is replaced as a whole. Everything else
/// is a binary we replace (portable, or inside the macOS `.app` bundle).
fn install_kind() -> InstallKind {
    if cfg!(windows) {
        return kind_for_location(
            std::env::current_exe().ok(),
            std::env::var_os("ProgramFiles"),
        );
    }
    if running_appimage().is_some() {
        return InstallKind::AppImage;
    }
    InstallKind::ReplaceBinary
}

/// The `.AppImage` file this process runs from, if any.
fn running_appimage() -> Option<PathBuf> {
    appimage_of(
        std::env::current_exe().ok()?,
        std::env::var_os("APPIMAGE")?,
        std::env::var_os("APPDIR")?,
    )
}

/// The AppImage runtime sets `APPIMAGE` (the file) and `APPDIR` (where the image is
/// mounted). Our children, the integrated terminal's shell included, inherit both: only
/// trust them when `exe` really lives in `APPDIR`.
fn appimage_of(
    exe: PathBuf,
    appimage: std::ffi::OsString,
    appdir: std::ffi::OsString,
) -> Option<PathBuf> {
    let inside = !appdir.is_empty() && exe.starts_with(&appdir);
    (inside && !appimage.is_empty()).then(|| PathBuf::from(appimage))
}

/// Windows: `Msi` when `exe` lives under `program_files`, else `ReplaceBinary`.
fn kind_for_location(
    exe: Option<PathBuf>,
    program_files: Option<std::ffi::OsString>,
) -> InstallKind {
    let in_program_files = exe
        .and_then(|exe| Some(exe.starts_with(program_files?)))
        .unwrap_or(false);
    if in_program_files {
        InstallKind::Msi
    } else {
        InstallKind::ReplaceBinary
    }
}

/// Returns the staged MSI path for MSI installs, `None` once the binary has been replaced.
fn download_and_apply(info: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
    let mut resp = http_get(&info.asset.url)?;
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_ASSET_BYTES)
        .read_to_vec()
        .map_err(|e| format!("downloading {}: {e}", info.asset.name))?;
    stage_and_apply(info, &bytes, &update_dir())
}

/// Verify `bytes`, write them into `dir`, then stage (MSI) or self-replace (binary).
fn stage_and_apply(
    info: &ReleaseInfo,
    bytes: &[u8],
    dir: &std::path::Path,
) -> Result<Option<PathBuf>, String> {
    verify_digest(bytes, &info.asset.sha256)?;

    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(&info.asset.name);
    std::fs::write(&path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;

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
        InstallKind::AppImage => {
            let res = running_appimage()
                .ok_or_else(|| "not running from an AppImage".to_string())
                .and_then(|appimage| replace_file(&path, &appimage));
            let _ = std::fs::remove_file(&path);
            res.map(|_| None)
        }
    }
}

/// Swap `target` for an executable copy of `source`. The copy is made next to `target` and
/// renamed over it, so the swap is atomic and the running AppImage (mounted from the old
/// file, still open) keeps working until the app exits.
fn replace_file(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let name = target
        .file_name()
        .ok_or_else(|| format!("bad AppImage path {}", target.display()))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".update");
    let tmp = target.with_file_name(tmp_name);
    let res = std::fs::copy(source, &tmp)
        .map_err(|e| format!("writing {}: {e}", tmp.display()))
        .and_then(|_| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
            }
            std::fs::rename(&tmp, target)
                .map_err(|e| format!("replacing {}: {e}", target.display()))
        });
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Refuse to install anything that doesn't match the manifest's checksum.
fn verify_digest(bytes: &[u8], expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Err("release asset has no sha256; refusing to install".into());
    }
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

    fn manifest(version: &str, assets: &[&str]) -> Manifest {
        Manifest {
            version: version.into(),
            notes: String::new(),
            page_url: String::new(),
            assets: assets
                .iter()
                .map(|n| ManifestAsset {
                    name: (*n).into(),
                    url: String::new(),
                    sha256: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn older_or_equal_release_is_not_an_update() {
        let cur = semver::Version::new(0, 5, 0);
        let m = manifest("0.5.0", &[MSI_ASSET]);
        assert!(select_update(m, &cur, InstallKind::Msi).unwrap().is_none());
        let m = manifest("0.4.9", &[MSI_ASSET]);
        assert!(select_update(m, &cur, InstallKind::Msi).unwrap().is_none());
    }

    #[test]
    fn newer_release_picks_matching_asset() {
        let cur = semver::Version::new(0, 5, 0);
        let m = manifest("v0.6.0", &["other", MSI_ASSET]);
        let info = select_update(m, &cur, InstallKind::Msi).unwrap().unwrap();
        assert_eq!(info.version, semver::Version::new(0, 6, 0));
        assert_eq!(info.asset.name, MSI_ASSET);
    }

    #[test]
    fn newer_release_without_asset_errors() {
        let cur = semver::Version::new(0, 5, 0);
        let m = manifest("0.6.0", &["other"]);
        assert!(select_update(m, &cur, InstallKind::Msi).is_err());
    }

    #[test]
    fn parses_manifest_written_by_release_workflow() {
        let json = r#"{
            "version": "0.6.0",
            "notes": "Bug fixes",
            "page_url": "https://ajustor.github.io/codingUnicorns/",
            "assets": [{
                "name": "coding-unicorns-setup.msi",
                "url": "https://ajustor.github.io/codingUnicorns/download/v0.6.0/coding-unicorns-setup.msi",
                "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            }]
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        let cur = semver::Version::new(0, 5, 0);
        let info = select_update(m, &cur, InstallKind::Msi).unwrap().unwrap();
        assert!(info.asset.url.ends_with("/coding-unicorns-setup.msi"));
    }

    #[test]
    fn digest_verification() {
        // sha256("abc")
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_digest(b"abc", abc).is_ok());
        assert!(verify_digest(b"abd", abc).is_err());
        assert!(verify_digest(b"abc", "").is_err());
    }

    #[test]
    fn digest_is_case_insensitive_and_reports_actual() {
        let upper = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert!(verify_digest(b"abc", upper).is_ok());
        let err = verify_digest(b"", "00").unwrap_err();
        // sha256("") is reported so a bad manifest can be diagnosed.
        assert!(err.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
        assert!(err.contains("expected 00"));
    }

    // ---- select_update / manifest edge cases ----

    #[test]
    fn select_update_copies_notes_page_and_kind() {
        let cur = semver::Version::new(0, 1, 0);
        let mut m = manifest("1.0.0", &[MSI_ASSET]);
        m.notes = "notes".into();
        m.page_url = "https://example.invalid/".into();
        let info = select_update(m, &cur, InstallKind::Msi).unwrap().unwrap();
        assert_eq!(info.notes, "notes");
        assert_eq!(info.page_url, "https://example.invalid/");
        assert_eq!(info.kind, InstallKind::Msi);
    }

    #[test]
    fn select_update_rejects_bad_version() {
        let cur = semver::Version::new(0, 1, 0);
        let err =
            select_update(manifest("latest", &[MSI_ASSET]), &cur, InstallKind::Msi).unwrap_err();
        assert!(err.contains("bad release version"), "{err}");
    }

    #[test]
    fn select_update_prerelease_ordering() {
        let cur = semver::Version::parse("1.0.0").unwrap();
        // 1.0.0-rc.1 < 1.0.0: not an update
        let m = manifest("1.0.0-rc.1", &[MSI_ASSET]);
        assert!(select_update(m, &cur, InstallKind::Msi).unwrap().is_none());
        let m = manifest("1.0.1-rc.1", &[MSI_ASSET]);
        assert!(select_update(m, &cur, InstallKind::Msi).unwrap().is_some());
    }

    #[test]
    fn select_update_binary_asset_for_this_platform() {
        let cur = semver::Version::new(0, 1, 0);
        match BINARY_ASSET {
            Some(name) => {
                let m = manifest("9.0.0", &[MSI_ASSET, name]);
                let info = select_update(m, &cur, InstallKind::ReplaceBinary)
                    .unwrap()
                    .unwrap();
                assert_eq!(info.asset.name, name);
                assert_eq!(info.kind, InstallKind::ReplaceBinary);
                // MSI alone is not enough for a portable install.
                let m = manifest("9.0.0", &[MSI_ASSET]);
                assert!(select_update(m, &cur, InstallKind::ReplaceBinary).is_err());
            }
            None => {
                let m = manifest("9.0.0", &[MSI_ASSET]);
                assert!(select_update(m, &cur, InstallKind::ReplaceBinary).is_err());
            }
        }
    }

    #[test]
    fn release_from_manifest_parses_and_compares_to_current() {
        let json = format!(
            r#"{{"version":"999.0.0","page_url":"p","assets":[{{"name":"{MSI_ASSET}","url":"u","sha256":"s"}}]}}"#
        );
        let info = release_from_manifest(&json, InstallKind::Msi)
            .unwrap()
            .unwrap();
        assert_eq!(info.version, semver::Version::new(999, 0, 0));
        assert_eq!(info.notes, "", "notes default to empty");
        assert_eq!(info.asset.url, "u");

        let same = format!(
            r#"{{"version":"{}","page_url":"p"}}"#,
            Updater::current_version()
        );
        assert!(release_from_manifest(&same, InstallKind::Msi)
            .unwrap()
            .is_none());
    }

    #[test]
    fn release_from_manifest_rejects_malformed() {
        let err = release_from_manifest("{not json", InstallKind::Msi).unwrap_err();
        assert!(err.starts_with("parsing release manifest"), "{err}");
        // page_url is required
        assert!(release_from_manifest(r#"{"version":"1.0.0"}"#, InstallKind::Msi).is_err());
    }

    #[test]
    fn current_version_is_semver() {
        assert!(semver::Version::parse(Updater::current_version()).is_ok());
    }

    // ---- install kind ----

    #[test]
    fn kind_for_location_detects_program_files() {
        // Built from components so the test holds on every platform's separator.
        let root = std::env::temp_dir();
        let pf_dir = root.join("Program Files");
        let pf = pf_dir.clone().into_os_string();
        assert_eq!(
            kind_for_location(
                Some(pf_dir.join("Coding Unicorns").join("cu.exe")),
                Some(pf.clone())
            ),
            InstallKind::Msi
        );
        assert_eq!(
            kind_for_location(Some(root.join("tools").join("cu.exe")), Some(pf.clone())),
            InstallKind::ReplaceBinary
        );
        // Path-component match, not string prefix.
        assert_eq!(
            kind_for_location(
                Some(root.join("Program Files (x86)").join("cu.exe")),
                Some(pf.clone())
            ),
            InstallKind::ReplaceBinary
        );
        assert_eq!(
            kind_for_location(None, Some(pf)),
            InstallKind::ReplaceBinary
        );
        assert_eq!(
            kind_for_location(Some(root.join("x").join("cu.exe")), None),
            InstallKind::ReplaceBinary
        );
    }

    #[test]
    fn test_binary_is_a_portable_install() {
        // The test executable lives under target/, never Program Files.
        assert_eq!(install_kind(), InstallKind::ReplaceBinary);
    }

    #[test]
    fn appimage_is_trusted_only_from_inside_its_mount() {
        let appimage = || std::ffi::OsString::from("/home/u/Apps/cu.AppImage");
        let appdir = || std::ffi::OsString::from("/tmp/.mount_cuAbc");
        let inside = PathBuf::from("/tmp/.mount_cuAbc/usr/bin/coding-unicorns");
        assert_eq!(
            appimage_of(inside.clone(), appimage(), appdir()),
            Some(PathBuf::from("/home/u/Apps/cu.AppImage"))
        );
        // Variables inherited by another build started from the integrated terminal.
        let elsewhere = PathBuf::from("/home/u/.cargo/bin/coding-unicorns");
        assert_eq!(appimage_of(elsewhere, appimage(), appdir()), None);
        // A component-wise prefix, not a string one.
        let sibling = PathBuf::from("/tmp/.mount_cuAbcd/usr/bin/coding-unicorns");
        assert_eq!(appimage_of(sibling, appimage(), appdir()), None);
        assert_eq!(appimage_of(inside.clone(), appimage(), "".into()), None);
        assert_eq!(appimage_of(inside, "".into(), appdir()), None);
    }

    #[test]
    fn select_update_appimage_asset_for_this_platform() {
        let cur = semver::Version::new(0, 1, 0);
        let m = manifest("9.0.0", &[MSI_ASSET, "coding-unicorns-linux-x64.AppImage"]);
        let res = select_update(m, &cur, InstallKind::AppImage);
        match APPIMAGE_ASSET {
            Some(name) => {
                let info = res.unwrap().unwrap();
                assert_eq!(info.asset.name, name);
                assert_eq!(info.kind, InstallKind::AppImage);
            }
            None => assert!(res.is_err()),
        }
    }

    #[test]
    fn replace_file_swaps_in_an_executable_copy() {
        let dir = scratch_dir("appimage");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("download");
        let target = dir.join("cu.AppImage");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_file(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        // Only the source and the replaced file are left: no temporary copy.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_reports_missing_directory() {
        let dir = scratch_dir("appimage-missing");
        let err = replace_file(&dir.join("src"), &dir.join("cu.AppImage")).unwrap_err();
        assert!(err.starts_with("writing"), "{err}");
    }

    // ---- stage_and_apply (MSI path only: never self-replace the test binary) ----

    fn scratch_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cu-updater-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn stage_msi_writes_verified_file() {
        let dir = scratch_dir("stage");
        let mut i = info("1.0.0", InstallKind::Msi);
        i.asset.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        let staged = stage_and_apply(&i, b"abc", &dir).unwrap().unwrap();
        assert_eq!(staged, dir.join(MSI_ASSET));
        assert_eq!(std::fs::read(&staged).unwrap(), b"abc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_refuses_checksum_mismatch_without_writing() {
        let dir = scratch_dir("mismatch");
        let mut i = info("1.0.0", InstallKind::Msi);
        i.asset.sha256 = "00".repeat(32);
        let err = stage_and_apply(&i, b"abc", &dir).unwrap_err();
        assert!(err.contains("checksum mismatch"));
        assert!(!dir.exists(), "nothing written before verification");
        // Even a ReplaceBinary asset is refused before self_replace is reached.
        let mut i = info("1.0.0", InstallKind::ReplaceBinary);
        i.asset.sha256 = String::new();
        assert!(stage_and_apply(&i, b"abc", &dir).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn stage_reports_unwritable_dir() {
        // A regular file where the directory should be.
        let file = scratch_dir("notadir");
        std::fs::write(&file, b"x").unwrap();
        let mut i = info("1.0.0", InstallKind::Msi);
        i.asset.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        let err = stage_and_apply(&i, b"abc", &file).unwrap_err();
        assert!(err.starts_with("creating"), "{err}");
        let _ = std::fs::remove_file(&file);
    }

    // ---- Updater state machine ----

    fn info(version: &str, kind: InstallKind) -> ReleaseInfo {
        ReleaseInfo {
            version: semver::Version::parse(version).unwrap(),
            notes: String::new(),
            page_url: String::new(),
            asset: ManifestAsset {
                name: MSI_ASSET.into(),
                url: String::new(),
                sha256: String::new(),
            },
            kind,
        }
    }

    fn send(u: &Updater, msg: Msg) {
        u.tx.send(msg).unwrap();
    }

    /// Poll until the worker result has been consumed (state leaves Checking/Downloading).
    fn wait(u: &mut Updater) -> Option<UpdateEvent> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let ev = u.poll();
            if !u.is_busy() {
                return ev;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("worker never reported");
    }

    #[test]
    fn new_updater_is_idle() {
        let mut u = Updater::default();
        assert!(matches!(u.state, UpdateState::Idle));
        assert!(!u.dismissed);
        assert!(!u.is_busy());
        assert!(u.exit_action.is_none());
        assert!(u.poll().is_none(), "no message, no event");
    }

    #[test]
    fn is_busy_only_while_checking_or_downloading() {
        let mut u = Updater::new();
        for (state, busy) in [
            (UpdateState::Idle, false),
            (UpdateState::Checking, true),
            (UpdateState::UpToDate, false),
            (
                UpdateState::Available(info("1.0.0", InstallKind::Msi)),
                false,
            ),
            (
                UpdateState::Downloading(info("1.0.0", InstallKind::Msi)),
                true,
            ),
            (UpdateState::Ready(info("1.0.0", InstallKind::Msi)), false),
            (UpdateState::Failed("x".into()), false),
        ] {
            u.state = state;
            assert_eq!(u.is_busy(), busy, "{:?}", u.state);
        }
    }

    #[test]
    fn poll_checked_available() {
        let mut u = Updater::new();
        u.state = UpdateState::Checking;
        send(&u, Msg::Checked(Ok(Some(info("2.0.0", InstallKind::Msi)))));
        match u.poll() {
            Some(UpdateEvent::Available(v)) => assert_eq!(v, semver::Version::new(2, 0, 0)),
            _ => panic!("expected Available"),
        }
        assert!(matches!(&u.state, UpdateState::Available(i) if i.version.major == 2));
    }

    #[test]
    fn poll_up_to_date_reports_only_when_manual() {
        let mut u = Updater::new();
        send(&u, Msg::Checked(Ok(None)));
        assert!(u.poll().is_none());
        assert!(matches!(u.state, UpdateState::UpToDate));

        u.manual = true;
        send(&u, Msg::Checked(Ok(None)));
        assert!(matches!(u.poll(), Some(UpdateEvent::UpToDate)));
    }

    #[test]
    fn poll_check_error_reports_only_when_manual() {
        let mut u = Updater::new();
        send(&u, Msg::Checked(Err("offline".into())));
        assert!(u.poll().is_none());
        assert!(matches!(&u.state, UpdateState::Failed(e) if e == "offline"));

        u.manual = true;
        send(&u, Msg::Checked(Err("boom".into())));
        assert!(matches!(u.poll(), Some(UpdateEvent::Error(e)) if e == "boom"));
        assert!(matches!(&u.state, UpdateState::Failed(e) if e == "boom"));
    }

    #[test]
    fn poll_installed_binary_becomes_ready() {
        let mut u = Updater::new();
        u.state = UpdateState::Downloading(info("2.0.0", InstallKind::ReplaceBinary));
        send(&u, Msg::Installed(Ok(None)));
        assert!(matches!(u.poll(), Some(UpdateEvent::Ready)));
        assert!(matches!(&u.state, UpdateState::Ready(i) if i.version.major == 2));
        assert!(u.staged_msi.is_none());
    }

    #[test]
    fn poll_installed_msi_is_staged() {
        let mut u = Updater::new();
        u.state = UpdateState::Downloading(info("2.0.0", InstallKind::Msi));
        send(&u, Msg::Installed(Ok(Some(PathBuf::from("setup.msi")))));
        assert!(matches!(u.poll(), Some(UpdateEvent::Ready)));
        assert_eq!(
            u.staged_msi.as_deref(),
            Some(std::path::Path::new("setup.msi"))
        );
        // Installed when the app quits, even if the dialog is never answered.
        assert!(
            matches!(&u.exit_action, Some(ExitAction::RunMsi(p)) if p == &PathBuf::from("setup.msi"))
        );
    }

    #[test]
    fn poll_install_error_always_reported() {
        let mut u = Updater::new();
        assert!(!u.manual);
        u.state = UpdateState::Downloading(info("2.0.0", InstallKind::Msi));
        send(&u, Msg::Installed(Err("disk full".into())));
        assert!(matches!(u.poll(), Some(UpdateEvent::Error(e)) if e == "disk full"));
        assert!(matches!(&u.state, UpdateState::Failed(e) if e == "disk full"));
    }

    #[test]
    fn poll_installed_without_download_is_ignored() {
        let mut u = Updater::new();
        u.state = UpdateState::UpToDate;
        send(&u, Msg::Installed(Ok(None)));
        assert!(u.poll().is_none());
        // The stray result resets to Idle rather than claiming Ready.
        assert!(matches!(u.state, UpdateState::Idle));
        assert!(u.staged_msi.is_none());
    }

    #[test]
    fn poll_drains_one_message_per_call() {
        let mut u = Updater::new();
        u.manual = true;
        send(&u, Msg::Checked(Ok(None)));
        send(&u, Msg::Checked(Err("e".into())));
        assert!(matches!(u.poll(), Some(UpdateEvent::UpToDate)));
        assert!(matches!(u.poll(), Some(UpdateEvent::Error(_))));
        assert!(u.poll().is_none());
    }

    #[test]
    fn schedule_restart_requires_ready() {
        let mut u = Updater::new();
        u.state = UpdateState::Available(info("2.0.0", InstallKind::Msi));
        u.schedule_restart();
        assert!(u.exit_action.is_none());

        u.state = UpdateState::Ready(info("2.0.0", InstallKind::ReplaceBinary));
        u.schedule_restart();
        assert!(matches!(u.exit_action, Some(ExitAction::Relaunch)));

        u.staged_msi = Some(PathBuf::from("a.msi"));
        u.schedule_restart();
        assert!(
            matches!(&u.exit_action, Some(ExitAction::RunMsiThenRelaunch(p)) if p == &PathBuf::from("a.msi")),
            "Restart now must relaunch after the MSI"
        );
    }

    #[test]
    fn encode_powershell_command_is_base64_utf16le() {
        // "ab" -> UTF-16LE 61 00 62 00
        assert_eq!(encode_powershell_command("ab"), "YQBiAA==");
        assert_eq!(encode_powershell_command("a"), "YQA=");
        assert_eq!(encode_powershell_command(""), "");
        // 3 UTF-16 units = 6 bytes: no padding
        assert_eq!(encode_powershell_command("abc"), "YQBiAGMA");
    }

    #[test]
    fn msi_relaunch_script_waits_then_relaunches_with_workspace() {
        let s = msi_relaunch_script(
            std::path::Path::new(r"C:\Temp\cu.msi"),
            std::path::Path::new(r"C:\Program Files\Coding Unicorns\cu.exe"),
            Some(std::path::Path::new(r"C:\dev\it's mine")),
        );
        // `\` only separates paths on Windows, where this runs.
        let log = msi_log_path(std::path::Path::new(r"C:\Temp\cu.msi"));
        if cfg!(windows) {
            assert_eq!(log, std::path::Path::new(r"C:\Temp\install.log"));
        }
        assert!(
            s.contains(&format!(
                r#"-ArgumentList '/i','"C:\Temp\cu.msi"','/passive','/l*v','"{}"' -Wait -PassThru"#,
                log.display()
            )),
            "{s}"
        );
        assert!(
            s.contains(r"-FilePath 'C:\Program Files\Coding Unicorns\cu.exe' -ArgumentList"),
            "{s}"
        );
        // A new window, never handed to another running instance; single quotes
        // are doubled inside the PowerShell literal.
        assert!(
            s.contains(r#"-ArgumentList '--new-window','"C:\dev\it''s mine"'"#),
            "{s}"
        );
        assert!(s.contains("-eq 3010"));

        let s = msi_relaunch_script(
            std::path::Path::new("a.msi"),
            std::path::Path::new("cu.exe"),
            None,
        );
        assert!(
            s.contains("Start-Process -FilePath 'cu.exe' -ArgumentList '--new-window' }"),
            "{s}"
        );
    }

    #[test]
    fn schedule_on_quit_only_for_staged_msi() {
        let mut u = Updater::new();
        u.state = UpdateState::Ready(info("2.0.0", InstallKind::ReplaceBinary));
        u.schedule_on_quit();
        assert!(
            u.exit_action.is_none(),
            "binary updates need nothing on quit"
        );

        u.staged_msi = Some(PathBuf::from("b.msi"));
        u.state = UpdateState::Failed("x".into());
        u.schedule_on_quit();
        assert!(u.exit_action.is_none(), "not Ready");

        u.state = UpdateState::Ready(info("2.0.0", InstallKind::Msi));
        u.schedule_on_quit();
        assert!(
            matches!(&u.exit_action, Some(ExitAction::RunMsi(p)) if p == &PathBuf::from("b.msi"))
        );
    }

    fn fetch_v2() -> Result<Option<ReleaseInfo>, String> {
        Ok(Some(info("2.0.0", InstallKind::Msi)))
    }

    fn fetch_none() -> Result<Option<ReleaseInfo>, String> {
        Ok(None)
    }

    fn fetch_err() -> Result<Option<ReleaseInfo>, String> {
        Err("no network".into())
    }

    fn fetch_unreachable() -> Result<Option<ReleaseInfo>, String> {
        panic!("guarded check must not fetch")
    }

    #[test]
    fn check_finds_update_and_resets_dismissed() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.dismissed = true;
        u.check_with(false, None, &ctx, fetch_v2);
        assert!(matches!(u.state, UpdateState::Checking));
        assert!(!u.dismissed);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Available(_))));
        assert!(matches!(u.state, UpdateState::Available(_)));
    }

    #[test]
    fn automatic_check_filters_skipped_version() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.check_with(false, Some("2.0.0"), &ctx, fetch_v2);
        assert!(wait(&mut u).is_none(), "skipped silently");
        assert!(matches!(u.state, UpdateState::UpToDate));
    }

    #[test]
    fn manual_check_ignores_skipped_version() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.check_with(true, Some("2.0.0"), &ctx, fetch_v2);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Available(_))));
    }

    #[test]
    fn skipped_older_or_invalid_version_does_not_hide_update() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.check_with(false, Some("1.9.0"), &ctx, fetch_v2);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Available(_))));

        let mut u = Updater::new();
        u.check_with(false, Some("not-a-version"), &ctx, fetch_v2);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Available(_))));
    }

    #[test]
    fn manual_check_reports_up_to_date_and_errors() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.check_with(true, None, &ctx, fetch_none);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::UpToDate)));

        // Failed is not busy, so a new check may start.
        u.check_with(true, None, &ctx, fetch_err);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Error(e)) if e == "no network"));

        // A later automatic check clears the manual flag.
        u.check_with(false, None, &ctx, fetch_none);
        assert!(wait(&mut u).is_none());
    }

    #[test]
    fn check_is_ignored_while_busy_or_ready() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        for state in [
            UpdateState::Checking,
            UpdateState::Downloading(info("2.0.0", InstallKind::Msi)),
            UpdateState::Ready(info("2.0.0", InstallKind::Msi)),
        ] {
            u.state = state;
            u.dismissed = true;
            u.check_with(true, None, &ctx, fetch_unreachable);
            assert!(u.dismissed, "guarded check must not touch state");
            assert!(!u.manual);
        }
        assert!(matches!(u.state, UpdateState::Ready(_)));
        // Public entry point honours the same guard (and so never hits the network here).
        u.check(true, None, &ctx);
        assert!(matches!(u.state, UpdateState::Ready(_)));
    }

    fn apply_ok_binary(_: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }

    fn apply_ok_msi(i: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
        Ok(Some(PathBuf::from(&i.asset.name)))
    }

    fn apply_err(_: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
        Err("checksum mismatch".into())
    }

    fn apply_unreachable(_: &ReleaseInfo) -> Result<Option<PathBuf>, String> {
        panic!("guarded install must not download")
    }

    #[test]
    fn install_binary_then_restart_relaunches() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.state = UpdateState::Available(info("2.0.0", InstallKind::ReplaceBinary));
        u.install_with(&ctx, apply_ok_binary);
        assert!(matches!(u.state, UpdateState::Downloading(_)));
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Ready)));
        u.schedule_restart();
        assert!(matches!(u.exit_action, Some(ExitAction::Relaunch)));
    }

    #[test]
    fn install_msi_then_quit_runs_installer() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.state = UpdateState::Available(info("2.0.0", InstallKind::Msi));
        u.install_with(&ctx, apply_ok_msi);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Ready)));
        u.schedule_on_quit();
        assert!(
            matches!(&u.exit_action, Some(ExitAction::RunMsi(p)) if p == &PathBuf::from(MSI_ASSET))
        );
    }

    #[test]
    fn install_failure_is_reported() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        u.state = UpdateState::Available(info("2.0.0", InstallKind::Msi));
        u.install_with(&ctx, apply_err);
        assert!(matches!(wait(&mut u), Some(UpdateEvent::Error(e)) if e == "checksum mismatch"));
        assert!(matches!(u.state, UpdateState::Failed(_)));
        u.schedule_restart();
        assert!(u.exit_action.is_none());
    }

    #[test]
    fn install_requires_available() {
        let ctx = egui::Context::default();
        let mut u = Updater::new();
        for state in [
            UpdateState::Idle,
            UpdateState::Checking,
            UpdateState::UpToDate,
            UpdateState::Downloading(info("2.0.0", InstallKind::Msi)),
            UpdateState::Ready(info("2.0.0", InstallKind::Msi)),
            UpdateState::Failed("x".into()),
        ] {
            let before = format!("{state:?}");
            u.state = state;
            u.install_with(&ctx, apply_unreachable);
            assert_eq!(format!("{:?}", u.state), before);
        }
        // Public entry point honours the same guard.
        u.install(&ctx);
        assert!(matches!(u.state, UpdateState::Failed(_)));
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// Hits the live manifest: `cargo test -- --ignored live_manifest`.
    #[test]
    #[ignore]
    fn live_manifest_has_installable_assets() {
        let mut resp = http_get(MANIFEST_URL).unwrap();
        let m: Manifest = serde_json::from_str(&resp.body_mut().read_to_string().unwrap()).unwrap();
        let old = semver::Version::new(0, 0, 1);
        for kind in [InstallKind::ReplaceBinary, InstallKind::Msi] {
            let info = select_update(m.clone(), &old, kind).unwrap().unwrap();
            assert_eq!(info.asset.sha256.len(), 64);
        }
    }
}
