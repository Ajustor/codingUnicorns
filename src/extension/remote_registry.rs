//! Remote module registry published on GitHub Pages.
//!
//! The registry is a single `registry.json` index listing every module with one
//! pre-built ZIP per platform. Each ZIP holds `<dir>/manifest.toml` plus that
//! platform's library — the layout the ZIP installer already accepts.
//!
//! Network and disk work never runs on the UI thread: `start_fetch_index` and
//! `start_install` spawn a worker and return a channel the UI polls each frame.
//! The HTTP layer is injected (`FetchFn`) so tests never touch the network.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::installer::{WorkspaceStatus, ZipInstallOptions};
use super::manifest::{Dependencies, ExtensionSource, SourceKind};

/// Index published by the `coding-unicorns-modules` repository.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://ajustor.github.io/coding-unicorns-modules/registry.json";
/// Default before the modules repository was renamed. GitHub doesn't redirect Pages
/// after a rename, so a config still holding it is moved to [`DEFAULT_REGISTRY_URL`].
pub const LEGACY_REGISTRY_URL: &str =
    "https://ajustor.github.io/writing-unicorns-modules/registry.json";
/// Highest index `schema` this build understands.
pub const SUPPORTED_SCHEMA: u32 = 1;

const USER_AGENT: &str = concat!("coding-unicorns/", env!("CARGO_PKG_VERSION"));
/// Hard cap on the index size, as a guard against a runaway response.
pub const MAX_INDEX_BYTES: u64 = 2 * 1024 * 1024;
/// Hard cap on a module archive.
pub const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
const ASSET_TIMEOUT: Duration = Duration::from_secs(300);

/// `GET url`, reading at most `limit` bytes of body.
pub type FetchFn = fn(&str, u64) -> Result<Vec<u8>, String>;

// ── Index format ─────────────────────────────────────────────────────────────

/// `registry.json`. Unknown fields are ignored so the format can grow.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    pub schema: u32,
    /// Release tag of the modules repository the assets come from.
    #[serde(default)]
    pub release: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub modules: Vec<RegistryModule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryModule {
    /// Extension id (`manifest.toml` `extension.id`).
    pub id: String,
    /// Top-level directory of the module inside its ZIP.
    pub dir: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub lsp_server: Option<String>,
    #[serde(default)]
    pub dependencies: Dependencies,
    /// Platform key (see [`platform_key`]) → archive.
    #[serde(default)]
    pub assets: HashMap<String, RegistryAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryAsset {
    pub url: String,
    /// Lowercase hex SHA-256 of the archive.
    #[serde(default)]
    pub sha256: String,
    /// Archive size in bytes.
    #[serde(default)]
    pub size: Option<u64>,
}

/// Asset key for the running platform, e.g. `windows-x86_64`.
pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

impl RegistryIndex {
    pub fn module(&self, id: &str) -> Option<&RegistryModule> {
        self.modules.iter().find(|m| m.id == id)
    }
}

impl RegistryModule {
    pub fn asset_for(&self, platform: &str) -> Option<&RegistryAsset> {
        self.assets.get(platform)
    }

    /// Archive for the running platform, if one is published.
    pub fn current_asset(&self) -> Option<&RegistryAsset> {
        self.asset_for(&platform_key())
    }

    /// Case-insensitive match on name, id, description and languages.
    /// `query` must already be lowercase; an empty query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim();
        query.is_empty()
            || self.name.to_lowercase().contains(query)
            || self.id.to_lowercase().contains(query)
            || self.description.to_lowercase().contains(query)
            || self
                .languages
                .iter()
                .any(|l| l.to_lowercase().contains(query))
    }
}

/// What the picker offers for a registry module.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleAction {
    Install,
    /// Installed and up to date (or no newer build for this platform).
    Installed,
    /// A newer version is available for this platform.
    Update(String),
    NotAvailable,
}

/// Decide the row action from the installed version (if any).
pub fn module_action(module: &RegistryModule, installed_version: Option<&str>) -> ModuleAction {
    let available = module.current_asset().is_some();
    match installed_version {
        Some(cur) if available && super::registry::version_gt(&module.version, cur) => {
            ModuleAction::Update(module.version.clone())
        }
        Some(_) => ModuleAction::Installed,
        None if available => ModuleAction::Install,
        None => ModuleAction::NotAvailable,
    }
}

/// Parse `registry.json`, rejecting schemas newer than this build supports.
pub fn parse_index(text: &str) -> Result<RegistryIndex, String> {
    #[derive(Deserialize)]
    struct Probe {
        schema: u32,
    }
    // Check the schema first: a newer schema may not fit the structs below.
    let probe: Probe =
        serde_json::from_str(text).map_err(|e| format!("invalid registry index: {e}"))?;
    if probe.schema > SUPPORTED_SCHEMA {
        return Err(format!(
            "the module registry uses format v{} but this version of Coding Unicorns only \
             supports v{SUPPORTED_SCHEMA} — update the IDE to browse it",
            probe.schema
        ));
    }
    let mut index: RegistryIndex =
        serde_json::from_str(text).map_err(|e| format!("invalid registry index: {e}"))?;
    // Module ids become directory names; drop anything unsafe up front.
    index.modules.retain(|m| {
        let ok = super::manifest::validate_extension_id(&m.id).is_ok();
        if !ok {
            log::warn!("registry: skipping module with invalid id {:?}", m.id);
        }
        ok
    });
    Ok(index)
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

/// TLS settings of every HTTPS request: certificates are checked against
/// the operating system's trust store (macOS Keychain, Windows store, Linux
/// CA bundle) rather than the roots bundled in the binary, so a corporate
/// proxy or an antivirus inspecting HTTPS (whose root the user's system
/// trusts) no longer fails with "invalid peer certificate: UnknownIssuer".
pub fn tls_config() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build()
}

/// Real HTTP fetch via `ureq`, with timeouts and a body size cap.
pub fn http_fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let timeout = if limit > MAX_INDEX_BYTES {
        ASSET_TIMEOUT
    } else {
        INDEX_TIMEOUT
    };
    let mut config = ureq::Agent::config_builder()
        .tls_config(tls_config())
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(timeout));
    if is_loopback(url) {
        // Never route local servers through a proxy from the environment.
        config = config.proxy(None);
    }
    let agent: ureq::Agent = config.build().into();
    let mut resp = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("downloading {url}: {e}"))
}

fn is_loopback(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    rest.starts_with("127.0.0.1") || rest.starts_with("localhost") || rest.starts_with("[::1]")
}

/// Download and parse the index at `url`.
pub fn fetch_index_with(fetch: FetchFn, url: &str) -> Result<RegistryIndex, String> {
    let bytes = fetch(url, MAX_INDEX_BYTES)?;
    let text =
        String::from_utf8(bytes).map_err(|_| "registry index is not valid UTF-8".to_string())?;
    parse_index(&text)
}

/// Fetch the index on a background thread.
pub fn start_fetch_index(
    url: String,
    ctx: Option<egui::Context>,
) -> mpsc::Receiver<Result<RegistryIndex, String>> {
    start_fetch_index_with(http_fetch, url, ctx)
}

pub fn start_fetch_index_with(
    fetch: FetchFn,
    url: String,
    ctx: Option<egui::Context>,
) -> mpsc::Receiver<Result<RegistryIndex, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch_index_with(fetch, &url));
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    });
    rx
}

// ── Install ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum RegistryInstallStatus {
    Downloading,
    Installing,
    InstallingDep(String),
    /// Installed; `warnings` are non-fatal problems (e.g. a dependency failed).
    Done {
        warnings: Vec<String>,
    },
    Failed(String),
}

/// Refuse an archive whose size or SHA-256 differs from the index.
pub fn verify_asset(bytes: &[u8], asset: &RegistryAsset) -> Result<(), String> {
    if let Some(size) = asset.size {
        if bytes.len() as u64 != size {
            return Err(format!(
                "size mismatch (expected {size} bytes, got {})",
                bytes.len()
            ));
        }
    }
    if asset.sha256.is_empty() {
        return Err("registry asset has no sha256; refusing to install".into());
    }
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual.eq_ignore_ascii_case(asset.sha256.trim()) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch (expected {}, got {actual})",
            asset.sha256
        ))
    }
}

/// Download, verify and install `module` for this platform. Returns non-fatal
/// warnings on success. `index_url` is recorded in the module's `source.toml`.
pub fn install_module_with(
    fetch: FetchFn,
    index_url: &str,
    module: &RegistryModule,
    extensions_dir: &Path,
    progress: &dyn Fn(RegistryInstallStatus),
) -> Result<Vec<String>, String> {
    super::manifest::validate_extension_id(&module.id).map_err(|e| e.to_string())?;
    let asset = module.current_asset().ok_or_else(|| {
        format!(
            "{} is not available for this platform ({})",
            module.name,
            platform_key()
        )
    })?;
    progress(RegistryInstallStatus::Downloading);
    let bytes = fetch(&asset.url, MAX_ASSET_BYTES)?;
    verify_asset(&bytes, asset)?;

    progress(RegistryInstallStatus::Installing);
    let tmp = std::env::temp_dir().join(format!(
        "coding-unicorns-registry-{}.zip",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    let res = install_zip_file(&tmp, index_url, module, extensions_dir, progress);
    let _ = std::fs::remove_file(&tmp);
    res
}

/// Install `module` from an already verified archive on disk.
fn install_zip_file(
    zip_path: &Path,
    index_url: &str,
    module: &RegistryModule,
    extensions_dir: &Path,
    progress: &dyn Fn(RegistryInstallStatus),
) -> Result<Vec<String>, String> {
    let found = super::installer::discover_zip_modules(zip_path)
        .map_err(|e| format!("invalid archive: {e}"))?
        .into_iter()
        .find(|m| m.dir_name == module.dir)
        .ok_or_else(|| format!("archive has no {}/manifest.toml", module.dir))?;
    if found.manifest.extension.id != module.id {
        return Err(format!(
            "archive contains module {} but the registry lists {}",
            found.manifest.extension.id, module.id
        ));
    }

    let warnings = RefCell::new(Vec::new());
    let failure = RefCell::new(None);
    let installed = Cell::new(0);
    let source = |_: &str| ExtensionSource {
        kind: SourceKind::Registry,
        path: None,
        member: None,
        url: Some(index_url.to_string()),
        id: Some(module.id.clone()),
    };
    super::installer::zip_install_core(
        zip_path,
        extensions_dir,
        Some(std::slice::from_ref(&module.dir)),
        &ZipInstallOptions {
            source: &source,
            install_deps: true,
        },
        &|status| match status {
            WorkspaceStatus::InstallingDep { step, .. } => {
                progress(RegistryInstallStatus::InstallingDep(step))
            }
            WorkspaceStatus::ModuleFailed { reason, .. } => warnings.borrow_mut().push(reason),
            WorkspaceStatus::Failed(e) => *failure.borrow_mut() = Some(e),
            WorkspaceStatus::Done { installed: n, .. } => installed.set(n),
            _ => {}
        },
    );
    if let Some(e) = failure.into_inner() {
        return Err(e);
    }
    let warnings = warnings.into_inner();
    if installed.get() == 0 {
        return Err(if warnings.is_empty() {
            "module was not installed".to_string()
        } else {
            warnings.join("; ")
        });
    }
    Ok(warnings)
}

/// Install `module` on a background thread, streaming progress.
pub fn start_install(
    index_url: String,
    module: RegistryModule,
    extensions_dir: PathBuf,
    ctx: Option<egui::Context>,
) -> mpsc::Receiver<RegistryInstallStatus> {
    start_install_with(http_fetch, index_url, module, extensions_dir, ctx)
}

pub fn start_install_with(
    fetch: FetchFn,
    index_url: String,
    module: RegistryModule,
    extensions_dir: PathBuf,
    ctx: Option<egui::Context>,
) -> mpsc::Receiver<RegistryInstallStatus> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let repaint = || {
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        };
        let progress = |s: RegistryInstallStatus| {
            let _ = tx.send(s);
            repaint();
        };
        let final_status =
            match install_module_with(fetch, &index_url, &module, &extensions_dir, &progress) {
                Ok(warnings) => RegistryInstallStatus::Done { warnings },
                Err(e) => {
                    log::warn!("registry install of {} failed: {e}", module.id);
                    RegistryInstallStatus::Failed(e)
                }
            };
        progress(final_status);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real HTTPS through the system trust store (network needed).
    #[test]
    #[ignore = "network"]
    fn fetches_over_https_with_the_system_trust_store() {
        let body = http_fetch(DEFAULT_REGISTRY_URL, MAX_INDEX_BYTES).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"modules\""));
        let body = http_fetch(
            "https://ajustor.github.io/codingUnicorns/latest.json",
            MAX_INDEX_BYTES,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"version\""));
    }
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn lib_name() -> String {
        format!(
            "{}x.{}",
            if cfg!(windows) { "" } else { "lib" },
            std::env::consts::DLL_EXTENSION
        )
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn manifest_toml(id: &str, version: &str) -> String {
        format!(
            "[extension]\nid = \"{id}\"\nname = \"Rust\"\nversion = \"{version}\"\ndescription = \"d\"\n\n[capabilities]\nlanguages = [\"rs\"]\n"
        )
    }

    /// ZIP with `<dir>/manifest.toml` and a fake library for this platform.
    fn module_zip(dir: &str, manifest: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file(format!("{dir}/manifest.toml"), opts).unwrap();
            w.write_all(manifest.as_bytes()).unwrap();
            w.start_file(format!("{dir}/{}", lib_name()), opts).unwrap();
            w.write_all(b"not really a library").unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn module(assets: &[(&str, RegistryAsset)]) -> RegistryModule {
        RegistryModule {
            id: "unicorns.rust-lang".into(),
            dir: "rust-lang".into(),
            name: "Rust".into(),
            version: "0.2.0".into(),
            description: "Rust language support".into(),
            author: String::new(),
            languages: vec!["rs".into()],
            lsp_server: Some("rust-analyzer".into()),
            dependencies: Dependencies::default(),
            assets: assets
                .iter()
                .map(|(k, a)| (k.to_string(), a.clone()))
                .collect(),
        }
    }

    fn asset_for_bytes(url: &str, bytes: &[u8]) -> RegistryAsset {
        RegistryAsset {
            url: url.into(),
            sha256: sha256_hex(bytes),
            size: Some(bytes.len() as u64),
        }
    }

    const FULL_INDEX: &str = r#"{
        "schema": 1,
        "release": "v0.4.0",
        "generated_at": "2026-10-06T12:00:00Z",
        "future_top_level": {"anything": [1, 2]},
        "modules": [
            {
                "id": "unicorns.rust-lang",
                "dir": "rust-lang",
                "name": "Rust",
                "version": "0.2.0",
                "description": "Rust language support",
                "author": "Writing Unicorns",
                "languages": ["rs"],
                "lsp_server": "rust-analyzer",
                "dependencies": { "npm": [], "pip": [], "cargo": ["x"], "go": [], "dotnet": [] },
                "homepage": "ignored",
                "assets": {
                    "windows-x86_64": { "url": "https://h/w.zip", "sha256": "aa", "size": 10, "extra": true },
                    "linux-x86_64":   { "url": "https://h/l.zip", "sha256": "bb", "size": 1 }
                }
            },
            {
                "id": "unicorns.minimal",
                "dir": "minimal",
                "name": "Minimal",
                "version": "1.0.0",
                "lsp_server": null
            }
        ]
    }"#;

    #[test]
    fn parses_index_ignoring_unknown_fields() {
        let idx = parse_index(FULL_INDEX).unwrap();
        assert_eq!(idx.schema, 1);
        assert_eq!(idx.release, "v0.4.0");
        assert_eq!(idx.generated_at, "2026-10-06T12:00:00Z");
        assert_eq!(idx.modules.len(), 2);
        let rust = idx.module("unicorns.rust-lang").unwrap();
        assert_eq!(rust.dir, "rust-lang");
        assert_eq!(rust.author, "Writing Unicorns");
        assert_eq!(rust.lsp_server.as_deref(), Some("rust-analyzer"));
        assert_eq!(rust.dependencies.cargo, vec!["x"]);
        let w = rust.asset_for("windows-x86_64").unwrap();
        assert_eq!(w.url, "https://h/w.zip");
        assert_eq!(w.size, Some(10));
        assert!(rust.asset_for("macos-aarch64").is_none());
    }

    #[test]
    fn missing_optional_fields_default() {
        let idx = parse_index(FULL_INDEX).unwrap();
        let m = idx.module("unicorns.minimal").unwrap();
        assert!(m.description.is_empty() && m.author.is_empty());
        assert!(m.languages.is_empty());
        assert!(m.lsp_server.is_none());
        assert!(m.dependencies.npm.is_empty());
        assert!(m.assets.is_empty());
        assert!(idx.module("nope").is_none());

        let idx = parse_index(r#"{"schema": 1}"#).unwrap();
        assert!(idx.modules.is_empty() && idx.release.is_empty());
    }

    #[test]
    fn newer_schema_asks_to_update_the_ide() {
        // Even if the rest of the document no longer fits our structs.
        let err = parse_index(r#"{"schema": 2, "modules": "a different shape"}"#).unwrap_err();
        assert!(err.contains("update the IDE"), "{err}");
    }

    #[test]
    fn malformed_index_is_rejected() {
        assert!(parse_index("").is_err());
        assert!(parse_index("{}").is_err(), "schema is required");
        assert!(parse_index(r#"{"schema": 1, "modules": [{"id": "x"}]}"#).is_err());
    }

    #[test]
    fn modules_with_unsafe_ids_are_dropped() {
        let idx = parse_index(
            r#"{"schema":1,"modules":[
                {"id":"../evil","dir":"e","name":"E","version":"1"},
                {"id":"ok.mod","dir":"o","name":"O","version":"1"}]}"#,
        )
        .unwrap();
        let ids: Vec<&str> = idx.modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["ok.mod"]);
    }

    #[test]
    fn platform_asset_selection() {
        let key = platform_key();
        assert_eq!(
            key,
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
        );
        let a = asset_for_bytes("u-current", b"x");
        let other = asset_for_bytes("u-other", b"y");
        let m = module(&[(key.as_str(), a), ("plan9-mips", other)]);
        assert_eq!(m.current_asset().unwrap().url, "u-current");
        let m = module(&[("plan9-mips", asset_for_bytes("u", b""))]);
        assert!(m.current_asset().is_none());
    }

    #[test]
    fn module_action_variants() {
        let key = platform_key();
        let m = module(&[(key.as_str(), asset_for_bytes("u", b""))]);
        assert_eq!(module_action(&m, None), ModuleAction::Install);
        assert_eq!(module_action(&m, Some("0.2.0")), ModuleAction::Installed);
        assert_eq!(module_action(&m, Some("0.3.0")), ModuleAction::Installed);
        assert_eq!(
            module_action(&m, Some("0.1.9")),
            ModuleAction::Update("0.2.0".into())
        );
        let unavailable = module(&[]);
        assert_eq!(
            module_action(&unavailable, None),
            ModuleAction::NotAvailable
        );
        assert_eq!(
            module_action(&unavailable, Some("0.1.0")),
            ModuleAction::Installed,
            "no build to update to"
        );
    }

    #[test]
    fn search_matches_name_id_description_and_languages() {
        let m = module(&[]);
        for q in ["", "rust", "unicorns.", "language support", "rs", "  rust "] {
            assert!(m.matches(q), "{q:?}");
        }
        assert!(!m.matches("python"));
    }

    #[test]
    fn verify_asset_rejects_size_and_checksum_mismatch() {
        let data = b"payload";
        let good = asset_for_bytes("u", data);
        assert!(verify_asset(data, &good).is_ok());

        let upper = RegistryAsset {
            sha256: good.sha256.to_uppercase(),
            ..good.clone()
        };
        assert!(verify_asset(data, &upper).is_ok());

        let wrong_size = RegistryAsset {
            size: Some(3),
            ..good.clone()
        };
        let err = verify_asset(data, &wrong_size).unwrap_err();
        assert!(err.contains("size mismatch"), "{err}");

        let wrong_sum = RegistryAsset {
            sha256: "00".repeat(32),
            ..good.clone()
        };
        let err = verify_asset(data, &wrong_sum).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");

        let no_sum = RegistryAsset {
            sha256: String::new(),
            ..good.clone()
        };
        assert!(verify_asset(data, &no_sum).is_err());

        let no_size = RegistryAsset { size: None, ..good };
        assert!(verify_asset(data, &no_size).is_ok());
    }

    #[test]
    fn is_loopback_detection() {
        assert!(is_loopback("http://127.0.0.1:8080/x"));
        assert!(is_loopback("http://localhost/x"));
        assert!(!is_loopback("https://ajustor.github.io/x"));
    }

    // ── Install via injected fetchers ────────────────────────────────────

    fn fetch_corrupted(_url: &str, _limit: u64) -> Result<Vec<u8>, String> {
        Ok(b"tampered".to_vec())
    }

    fn fetch_offline(url: &str, _limit: u64) -> Result<Vec<u8>, String> {
        Err(format!("request to {url} failed: offline"))
    }

    #[test]
    fn install_rejects_tampered_download_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = module_zip("rust-lang", &manifest_toml("unicorns.rust-lang", "0.2.0"));
        let key = platform_key();
        let m = module(&[(key.as_str(), asset_for_bytes("https://h/x.zip", &zip))]);
        let err = install_module_with(fetch_corrupted, "idx", &m, tmp.path(), &|_| {}).unwrap_err();
        assert!(err.contains("mismatch"), "{err}");
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());

        let err = install_module_with(fetch_offline, "idx", &m, tmp.path(), &|_| {}).unwrap_err();
        assert!(err.contains("offline"), "{err}");
    }

    #[test]
    fn install_requires_an_asset_for_this_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let m = module(&[("plan9-mips", asset_for_bytes("u", b""))]);
        let err = install_module_with(fetch_offline, "idx", &m, tmp.path(), &|_| {}).unwrap_err();
        assert!(err.contains("not available for this platform"), "{err}");
    }

    #[test]
    fn install_zip_file_checks_dir_and_id() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("m.zip");
        let exts = tmp.path().join("exts");
        let m = module(&[]);

        std::fs::write(
            &zip_path,
            module_zip("other-dir", &manifest_toml("unicorns.rust-lang", "0.2.0")),
        )
        .unwrap();
        let err = install_zip_file(&zip_path, "idx", &m, &exts, &|_| {}).unwrap_err();
        assert!(err.contains("rust-lang/manifest.toml"), "{err}");

        std::fs::write(
            &zip_path,
            module_zip("rust-lang", &manifest_toml("acme.impostor", "0.2.0")),
        )
        .unwrap();
        let err = install_zip_file(&zip_path, "idx", &m, &exts, &|_| {}).unwrap_err();
        assert!(err.contains("acme.impostor"), "{err}");
        assert!(!exts.join("acme.impostor").exists());

        std::fs::write(&zip_path, b"not a zip").unwrap();
        assert!(install_zip_file(&zip_path, "idx", &m, &exts, &|_| {}).is_err());
    }

    // ── End-to-end over a local HTTP server ──────────────────────────────

    /// Serve the routes (path → body) built by `routes(base_url)` on
    /// 127.0.0.1 for `requests` requests. Returns the base URL.
    fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>, requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&base);
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let Ok(mut stream) = stream else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&buf);
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, b)) => ("200 OK", b.clone()),
                    None => ("404 Not Found", b"missing".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        base
    }

    #[test]
    fn end_to_end_install_from_local_server() {
        let zip = module_zip("rust-lang", &manifest_toml("unicorns.rust-lang", "0.2.0"));
        // The index embeds the server address, so routes are built once bound.
        let base = serve(
            |base| {
                let mut assets = serde_json::Map::new();
                assets.insert(
                    platform_key(),
                    serde_json::json!({
                        "url": format!("{base}/rust-lang.zip"),
                        "sha256": sha256_hex(&zip),
                        "size": zip.len(),
                    }),
                );
                let index = serde_json::json!({
                    "schema": 1,
                    "release": "v0.4.0",
                    "modules": [{
                        "id": "unicorns.rust-lang",
                        "dir": "rust-lang",
                        "name": "Rust",
                        "version": "0.2.0",
                        "languages": ["rs"],
                        "assets": assets,
                    }]
                });
                vec![
                    ("/registry.json".to_string(), index.to_string().into_bytes()),
                    ("/rust-lang.zip".to_string(), zip.clone()),
                ]
            },
            2,
        );
        let index_url = format!("{base}/registry.json");

        let idx = fetch_index_with(http_fetch, &index_url).unwrap();
        let m = idx.module("unicorns.rust-lang").unwrap().clone();

        let tmp = tempfile::tempdir().unwrap();
        let rx = start_install_with(
            http_fetch,
            index_url.clone(),
            m,
            tmp.path().to_path_buf(),
            None,
        );
        let statuses: Vec<RegistryInstallStatus> = rx.iter().collect();
        assert_eq!(
            statuses,
            vec![
                RegistryInstallStatus::Downloading,
                RegistryInstallStatus::Installing,
                RegistryInstallStatus::Done { warnings: vec![] },
            ]
        );

        let dest = tmp.path().join("unicorns.rust-lang");
        assert!(dest.join("manifest.toml").exists());
        assert!(dest.join(lib_name()).exists());
        let src: ExtensionSource =
            toml::from_str(&std::fs::read_to_string(dest.join("source.toml")).unwrap()).unwrap();
        assert_eq!(src.kind, SourceKind::Registry);
        assert_eq!(src.url.as_deref(), Some(index_url.as_str()));
        assert_eq!(src.id.as_deref(), Some("unicorns.rust-lang"));

        // The installed registry entry now round-trips through the loader.
        let mut reg = super::super::registry::ExtensionRegistry::new_in(tmp.path().to_path_buf());
        reg.load_installed();
        assert!(reg.is_installed("unicorns.rust-lang"));
    }

    #[test]
    fn http_fetch_enforces_limit_and_status() {
        let base = serve(|_| vec![("/big".to_string(), vec![b'x'; 4096])], 2);
        let err = http_fetch(&format!("{base}/big"), 100).unwrap_err();
        assert!(err.contains("downloading"), "{err}");
        let err = http_fetch(&format!("{base}/nope"), 100).unwrap_err();
        assert!(err.contains("failed"), "{err}");
    }
}
