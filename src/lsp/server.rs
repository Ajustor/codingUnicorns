//! Language servers declared by modules (`lsp_server` / `lsp_servers` in
//! their `manifest.toml`): the command line, with its variables expanded, and
//! the archive some servers download on first use (`lsp_download`), e.g. a
//! Java server run by `java -jar ${serverDir}/plugins/launcher_*.jar`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::dap::adapters::find_on_path;
use crate::extension::manifest::ArchiveDownload;
use crate::extension::registry::InstalledExtension;

/// The language server a module provides for one language.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleServer {
    /// Module name, for messages.
    pub module: String,
    module_id: String,
    /// Language (file extension or named language) the server is for.
    pub language: String,
    /// Folder of the installed module.
    dir: PathBuf,
    command: String,
    args: Vec<String>,
    download: Option<ArchiveDownload>,
    /// `initializationOptions`, variables not expanded yet.
    init_options: Option<toml::Table>,
    /// The module debugger's download (`[debugger.download]`), fetched with
    /// the server when the server uses its files (`${debuggerDir}`, e.g. a
    /// debug plugin the server loads).
    debugger_download: Option<ArchiveDownload>,
    /// A per-language server (`lsp_servers`): unpacked in its own folder.
    per_language: bool,
}

impl ModuleServer {
    /// The server for `language`, from the enabled module handling it.
    pub fn for_language<'a>(
        installed: impl IntoIterator<Item = &'a InstalledExtension>,
        language: &str,
    ) -> Option<Self> {
        installed.into_iter().filter(|e| e.enabled).find_map(|e| {
            let caps = &e.manifest.capabilities;
            let (command, args, download, per_language) = match caps.lsp_servers.get(language) {
                Some(s) => (s.command.clone(), s.args.clone(), s.download.clone(), true),
                None if caps.languages.iter().any(|l| l == language) => (
                    caps.lsp_server.clone()?,
                    caps.lsp_args.clone(),
                    caps.lsp_download.clone(),
                    false,
                ),
                None => return None,
            };
            let init_options = caps.lsp_init_options_for(language).cloned();
            let debugger_download = e
                .manifest
                .debugger
                .as_ref()
                .and_then(|d| d.download.clone());
            Some(Self {
                module: e.manifest.extension.name.clone(),
                module_id: e.manifest.extension.id.clone(),
                language: language.to_string(),
                dir: e.path.clone(),
                command,
                args,
                download,
                init_options,
                debugger_download,
                per_language,
            })
        })
    }

    /// Identifies the download (one per module, or per language server).
    pub fn download_key(&self) -> String {
        if self.per_language {
            format!("{}/{}", self.module_id, self.language)
        } else {
            self.module_id.clone()
        }
    }

    /// Folder the download is unpacked in (`${serverDir}`).
    pub fn server_dir(&self) -> PathBuf {
        let dir = self.dir.join("lsp");
        if self.per_language {
            dir.join(&self.language)
        } else {
            dir
        }
    }

    /// Whether the command line uses downloaded files.
    fn needs_files(&self) -> bool {
        std::iter::once(&self.command)
            .chain(&self.args)
            .any(|a| a.contains("${serverDir}"))
    }

    /// Folder of the module debugger's download (`${debuggerDir}`).
    fn debugger_dir(&self) -> PathBuf {
        self.dir.join("debugger")
    }

    /// The debugger's download when the server uses its files.
    fn debugger_files(&self) -> Option<&ArchiveDownload> {
        let download = self.debugger_download.as_ref()?;
        let init = self
            .init_options
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_default();
        let used = std::iter::once(&self.command)
            .chain(&self.args)
            .chain(std::iter::once(&init))
            .any(|a| a.contains("${debuggerDir}"));
        used.then_some(download)
    }

    /// Whether the debugger files the server uses still have to be fetched.
    fn needs_debugger_files(&self) -> bool {
        self.debugger_files().is_some_and(|d| {
            platform_url(d).is_some()
                && !marker_paths(&self.debugger_dir(), &d.binary)
                    .is_some_and(|m| m.iter().any(|p| is_installed(p)))
        })
    }

    /// The downloaded file marking the install, with or without the
    /// executable suffix. `None` without a download, or when the path would
    /// leave the module's folder.
    fn marker_candidates(&self) -> Option<[PathBuf; 2]> {
        marker_paths(&self.server_dir(), &self.download.as_ref()?.binary)
    }

    /// The marker file, when downloaded. A `*` in its name matches any
    /// file (a binary named after the platform, e.g. `lemminx-*`).
    fn installed_file(&self) -> Option<PathBuf> {
        self.marker_candidates()?
            .into_iter()
            .map(|p| {
                PathBuf::from(crate::extension::archive::resolve_glob(
                    &p.to_string_lossy(),
                ))
            })
            .find(|p| p.is_file())
    }

    /// Download URL for the running platform (`*` serves every platform).
    pub fn download_url(&self) -> Option<&str> {
        platform_url(self.download.as_ref()?)
    }

    fn needs_server_files(&self) -> bool {
        self.download_url().is_some()
            && self.installed_file().is_none()
            && (self.needs_files() || find_on_path(&self.command).is_none())
    }

    /// Whether files must be fetched before starting the server: its own
    /// archive, or the debugger files it uses.
    pub fn needs_download(&self) -> bool {
        self.needs_server_files() || self.needs_debugger_files()
    }

    /// Download what the server needs (blocking).
    pub fn download(&self) -> Result<(), String> {
        if self.needs_server_files() {
            let (Some(url), true) = (self.download_url(), self.marker_candidates().is_some())
            else {
                return Err("no download for this platform".into());
            };
            let binary = self.download.as_ref().map_or("", |d| d.binary.as_str());
            crate::extension::archive::download(url, &self.server_dir(), binary)?;
            if self.installed_file().is_none() {
                return Err(format!("{binary} not found in the downloaded archive"));
            }
        }
        if self.needs_debugger_files() {
            let d = self.debugger_files().expect("needed");
            if marker_paths(&self.debugger_dir(), &d.binary).is_none() {
                return Err(format!("invalid debugger file {}", d.binary));
            }
            let url = platform_url(d).expect("needed");
            crate::extension::archive::download(url, &self.debugger_dir(), &d.binary)?;
            if self.needs_debugger_files() {
                return Err(format!("{} not found in the debugger download", d.binary));
            }
        }
        if self.download_url().is_none() && self.debugger_files().is_none() {
            return Err("no download for this platform".into());
        }
        Ok(())
    }

    /// `initializationOptions` for `workspace`, with the variables of the
    /// command line (and `${debuggerDir}`) expanded in its strings.
    pub fn init_options(&self, workspace: &Path) -> Option<serde_json::Value> {
        let mut value = serde_json::to_value(self.init_options.as_ref()?).ok()?;
        let vars = self.vars(workspace);
        expand_strings(&mut value, &|s| expand_vars(s, &vars));
        Some(value)
    }

    /// Variable → value, for the command line and init options.
    fn vars(&self, workspace: &Path) -> [(&'static str, String); 4] {
        [
            (
                "${serverDir}",
                self.server_dir().to_string_lossy().into_owned(),
            ),
            (
                "${debuggerDir}",
                self.debugger_dir().to_string_lossy().into_owned(),
            ),
            (
                "${dataDir}",
                data_dir(&self.module_id, workspace)
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("${os}", os_name().to_string()),
        ]
    }

    /// Command and arguments to start the server for `workspace`, variables
    /// expanded. Creates the `${dataDir}` folder when it is used.
    pub fn command_line(&self, workspace: &Path) -> (String, Vec<String>) {
        if self.args.iter().any(|a| a.contains("${dataDir}")) {
            let _ = std::fs::create_dir_all(data_dir(&self.module_id, workspace));
        }
        let vars = self.vars(workspace);
        let expand = |s: &str| expand_vars(s, &vars);
        let mut command = with_exe_suffix(expand(&self.command));
        if !self.needs_files() && find_on_path(&command).is_none() {
            if let Some(file) = self.installed_file() {
                command = file.to_string_lossy().into_owned();
            }
        }
        (command, self.args.iter().map(|a| expand(a)).collect())
    }
}

/// `s` with the variables replaced, and a `*` file name resolved.
fn expand_vars(s: &str, vars: &[(&'static str, String)]) -> String {
    let s = vars
        .iter()
        .fold(s.to_string(), |s, (var, value)| s.replace(var, value));
    crate::extension::archive::resolve_glob(&s)
}

/// Apply `f` to every string in `value`.
fn expand_strings(value: &mut serde_json::Value, f: &dyn Fn(&str) -> String) {
    match value {
        serde_json::Value::String(s) => *s = f(s),
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| expand_strings(v, f)),
        serde_json::Value::Object(map) => map.values_mut().for_each(|v| expand_strings(v, f)),
        _ => {}
    }
}

/// `binary` under `dir`, with and without the executable suffix; `None` when
/// the path would leave `dir` (the manifest comes from the module).
fn marker_paths(dir: &Path, binary: &str) -> Option<[PathBuf; 2]> {
    let rel = Path::new(binary);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let plain = rel.components().fold(dir.to_path_buf(), |p, c| p.join(c));
    let mut exe = plain.clone();
    exe.as_mut_os_string().push(std::env::consts::EXE_SUFFIX);
    Some([exe, plain])
}

/// Whether a marker file (a `*` in its name matching any file) is there.
fn is_installed(marker: &Path) -> bool {
    Path::new(&crate::extension::archive::resolve_glob(
        &marker.to_string_lossy(),
    ))
    .is_file()
}

/// A download's URL for the running platform (`*` serves every platform).
fn platform_url(download: &ArchiveDownload) -> Option<&str> {
    download
        .urls
        .get(&crate::extension::remote_registry::platform_key())
        .or_else(|| download.urls.get("*"))
        .map(String::as_str)
}

/// A command that names a file without its `.exe` (`${serverDir}/srv`) gets
/// it on Windows.
fn with_exe_suffix(command: String) -> String {
    let path = Path::new(&command);
    if path.is_file() || path.extension().is_some() || std::env::consts::EXE_SUFFIX.is_empty() {
        return command;
    }
    let exe = format!("{command}{}", std::env::consts::EXE_SUFFIX);
    if Path::new(&exe).is_file() {
        exe
    } else {
        command
    }
}

/// `${dataDir}`: a folder for a server's own data (indexes, project
/// metadata), one per module and workspace, outside the workspace.
pub fn data_dir(module_id: &str, workspace: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(workspace.to_string_lossy().as_bytes());
    let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    let name = workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dirs_next::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("coding-unicorns")
        .join("lsp-data")
        .join(module_id)
        .join(format!("{name}-{hex}"))
}

/// `${os}`: `win`, `mac` or `linux`.
fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win",
        "macos" => "mac",
        _ => "linux",
    }
}

/// Server downloads running in the background, one per [`ModuleServer::download_key`].
#[derive(Default)]
pub struct ServerDownloads {
    running: HashMap<String, (String, Receiver<Result<(), String>>)>,
    /// Downloads that failed, not retried until [`Self::clear_failed`].
    failed: HashMap<String, String>,
}

impl ServerDownloads {
    /// Start downloading `server`'s archive unless it is already running or
    /// failed. Returns whether a download was started.
    pub fn start(&mut self, server: &ModuleServer, waker: Option<egui::Context>) -> bool {
        let key = server.download_key();
        if self.running.contains_key(&key) || self.failed.contains_key(&key) {
            return false;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let job = server.clone();
        let spawned = std::thread::Builder::new()
            .name("lsp-download".into())
            .spawn(move || {
                let _ = tx.send(job.download());
                if let Some(ctx) = waker {
                    ctx.request_repaint();
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.running.insert(key, (server.module.clone(), rx));
        true
    }

    pub fn is_running(&self) -> bool {
        !self.running.is_empty()
    }

    /// Finished downloads: (module name, result).
    pub fn poll(&mut self) -> Vec<(String, Result<(), String>)> {
        let mut done = vec![];
        self.running
            .retain(|key, (module, rx)| match rx.try_recv() {
                Ok(result) => {
                    if let Err(e) = &result {
                        self.failed.insert(key.clone(), e.clone());
                    }
                    done.push((module.clone(), result));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    self.failed
                        .insert(key.clone(), "download thread ended".into());
                    done.push((module.clone(), Err("download thread ended".into())));
                    false
                }
            });
        done
    }

    /// Allow failed downloads to be tried again (e.g. on "Restart LSP").
    pub fn clear_failed(&mut self) {
        self.failed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::manifest::ExtensionManifest;

    fn module(dir: &Path, capabilities: &str) -> InstalledExtension {
        let manifest = ExtensionManifest::parse(&format!(
            r#"[extension]
id = "acme.java"
name = "Java"
version = "1.0.0"
description = ""
author = ""

[capabilities]
{capabilities}
"#
        ))
        .unwrap();
        InstalledExtension {
            manifest,
            path: dir.to_path_buf(),
            lib_path: None,
            enabled: true,
            source: None,
            update_available: None,
        }
    }

    #[test]
    fn debugger_files_used_by_the_server_come_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["java"]
lsp_server = "java"
lsp_init_options = { bundles = ["${debuggerDir}/debug-*.jar"], data = "${dataDir}" }

[debugger]
command = "start"
transport = "lsp"
download = { binary = "debug-1.0.jar", urls = { "*" = "https://example.invalid/debug-1.0.jar" } }
"#,
        );
        let s = ModuleServer::for_language([&m], "java").unwrap();
        assert!(s.needs_download(), "the debug plugin is missing");
        let jar = dir.path().join("debugger").join("debug-1.0.jar");
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        std::fs::write(&jar, "").unwrap();
        assert!(!s.needs_download());

        let ws = dir.path().join("ws");
        let init = s.init_options(&ws).unwrap();
        assert_eq!(
            Path::new(init["bundles"][0].as_str().unwrap()),
            jar,
            "glob resolved"
        );
        assert_eq!(
            init["data"],
            data_dir("acme.java", &ws).to_string_lossy().as_ref()
        );

        // Without a reference to ${debuggerDir}, the debugger is not the
        // server's business.
        let plain = module(
            dir.path(),
            r#"languages = ["java"]
lsp_server = "java"

[debugger]
command = "x"
download = { binary = "other.jar", urls = { "*" = "https://example.invalid/o.jar" } }
"#,
        );
        let s = ModuleServer::for_language([&plain], "java").unwrap();
        assert!(!s.needs_download());
        assert!(s.init_options(&ws).is_none());
    }

    const JAVA: &str = r#"languages = ["java"]
lsp_server = "java"
lsp_args = ["-Dconf=${serverDir}/config_${os}", "-jar", "${serverDir}/plugins/launcher_*.jar", "-data", "${dataDir}"]
lsp_download = { binary = "bin/jdtls", urls = { "*" = "https://example.invalid/jdtls.tar.gz" } }
"#;

    #[test]
    fn finds_the_server_of_a_language_among_enabled_modules() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = module(dir.path(), JAVA);
        let s = ModuleServer::for_language([&m], "java").unwrap();
        assert_eq!(s.module, "Java");
        assert_eq!(s.server_dir(), dir.path().join("lsp"));
        assert_eq!(s.download_key(), "acme.java");
        assert!(ModuleServer::for_language([&m], "kt").is_none());
        m.enabled = false;
        assert!(ModuleServer::for_language([&m], "java").is_none());

        let none = module(dir.path(), r#"languages = ["txt"]"#);
        assert!(
            ModuleServer::for_language([&none], "txt").is_none(),
            "no server"
        );
    }

    #[test]
    fn per_language_servers_get_their_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["a", "b"]
lsp_server = "srv-a"
[capabilities.lsp_servers.b]
command = "srv-b"
args = ["--stdio"]
download = { binary = "srv-b", urls = { "*" = "https://example.invalid/b.zip" } }
"#,
        );
        let a = ModuleServer::for_language([&m], "a").unwrap();
        assert_eq!(a.command_line(dir.path()), ("srv-a".to_string(), vec![]));
        assert!(!a.needs_download(), "no download declared");
        let b = ModuleServer::for_language([&m], "b").unwrap();
        assert_eq!(b.server_dir(), dir.path().join("lsp").join("b"));
        assert_eq!(b.download_key(), "acme.java/b");
        assert!(b.needs_download(), "not on PATH, not downloaded");
    }

    #[test]
    fn download_is_needed_until_the_marker_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(dir.path(), JAVA);
        let s = ModuleServer::for_language([&m], "java").unwrap();
        assert_eq!(
            s.download_url(),
            Some("https://example.invalid/jdtls.tar.gz")
        );
        // `java` may be on PATH, but the arguments use the downloaded files.
        assert!(s.needs_download());
        std::fs::create_dir_all(dir.path().join("lsp/bin")).unwrap();
        std::fs::write(dir.path().join("lsp/bin/jdtls"), "").unwrap();
        assert!(!s.needs_download());
    }

    #[test]
    fn command_line_expands_variables_and_globs() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("lsp/plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        for f in ["launcher_1.2.jar", "launcher_1.10.jar", "other.jar"] {
            std::fs::write(plugins.join(f), "").unwrap();
        }
        let m = module(dir.path(), JAVA);
        let s = ModuleServer::for_language([&m], "java").unwrap();
        let ws = dir.path().join("my-project");
        let (cmd, args) = s.command_line(&ws);
        assert_eq!(cmd, "java");
        let server_dir = dir.path().join("lsp").to_string_lossy().into_owned();
        assert_eq!(args[0], format!("-Dconf={server_dir}/config_{}", os_name()));
        // Name order: "launcher_1.2.jar" > "launcher_1.10.jar".
        assert_eq!(args[2], format!("{server_dir}/plugins/launcher_1.2.jar"));
        let data = data_dir("acme.java", &ws);
        assert_eq!(args[4], data.to_string_lossy());
        assert!(data.is_dir(), "created on use");
        assert!(data
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("my-project-"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn data_dirs_differ_per_workspace_and_module() {
        let a = data_dir("m", Path::new("/w/a"));
        assert_eq!(a, data_dir("m", Path::new("/w/a")));
        assert_ne!(a, data_dir("m", Path::new("/x/a")));
        assert_ne!(a, data_dir("n", Path::new("/w/a")));
    }

    #[test]
    fn glob_only_in_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x-1.jar"), "").unwrap();
        let d = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            crate::extension::archive::resolve_glob(&format!("{d}/x-*.jar")),
            format!("{d}/x-1.jar")
        );
        assert_eq!(
            crate::extension::archive::resolve_glob(&format!("{d}/y-*.jar")),
            format!("{d}/y-*.jar")
        );
        assert_eq!(
            crate::extension::archive::resolve_glob("no-separator-*"),
            "no-separator-*"
        );
        assert_eq!(
            crate::extension::archive::resolve_glob("/a*/b.jar"),
            "/a*/b.jar"
        );
        assert_eq!(
            crate::extension::archive::resolve_glob(&format!("{d}/x-1.jar")),
            format!("{d}/x-1.jar")
        );
    }

    #[test]
    fn marker_and_command_with_a_platform_specific_name() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["xml"]
lsp_server = "${serverDir}/lemminx-*"
lsp_download = { binary = "lemminx-*", urls = { "*" = "https://example.invalid/l.zip" } }
"#,
        );
        let s = ModuleServer::for_language([&m], "xml").unwrap();
        assert!(s.needs_download());
        let name = format!("lemminx-linux{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(dir.path().join("lsp")).unwrap();
        std::fs::write(dir.path().join("lsp").join(&name), "").unwrap();
        assert!(!s.needs_download(), "found through the glob");
        let (cmd, _) = s.command_line(dir.path());
        assert!(cmd.ends_with(&name), "{cmd}");
    }

    #[test]
    fn command_path_without_exe_suffix_finds_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["rs"]
lsp_server = "${serverDir}/rust-analyzer"
lsp_download = { binary = "rust-analyzer", urls = { "*" = "https://example.invalid/ra.gz" } }
"#,
        );
        let s = ModuleServer::for_language([&m], "rs").unwrap();
        let exe = dir
            .path()
            .join("lsp")
            .join(format!("rust-analyzer{}", std::env::consts::EXE_SUFFIX));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert!(!s.needs_download());
        let (cmd, _) = s.command_line(dir.path());
        assert_eq!(Path::new(&cmd), exe);
    }

    #[test]
    fn marker_cannot_leave_the_module_folder() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["java"]
lsp_server = "srv"
lsp_download = { binary = "../escape", urls = { "*" = "https://example.invalid/a.zip" } }
"#,
        );
        let s = ModuleServer::for_language([&m], "java").unwrap();
        assert!(s.marker_candidates().is_none());
        assert!(s.download().is_err());
    }

    /// End to end with a module's real language server (its archive
    /// downloaded when the module has one). `cargo test e2e_module_language_server
    /// -- --ignored --nocapture` with:
    /// - `CU_E2E_MANIFEST`: the module's manifest.toml
    /// - `CU_E2E_FILE`: a source file, in its project folder
    /// - `CU_E2E_HOVER` (optional): `line:column:text`, 0-based, where
    ///   hovering shows `text`; without it the server only has to initialize
    /// - `CU_E2E_MODULE_DIR` (optional): module folder kept between runs
    ///   (to download once), else a temporary one
    #[test]
    #[ignore]
    fn e2e_module_language_server_starts() {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let file = PathBuf::from(var("CU_E2E_FILE").unwrap());
        let project = file.parent().unwrap().to_path_buf();
        let tmp = tempfile::tempdir().unwrap();
        let dir = var("CU_E2E_MODULE_DIR").map_or_else(|| tmp.path().join("module"), PathBuf::from);
        let mut module = module(&dir, "");
        module.manifest = ExtensionManifest::parse(
            &std::fs::read_to_string(var("CU_E2E_MANIFEST").unwrap()).unwrap(),
        )
        .unwrap();
        let ext = file.extension().unwrap().to_string_lossy().to_string();
        let server = ModuleServer::for_language([&module], &ext).expect("module server");
        if server.needs_download() {
            eprintln!("downloading {:?}", server.download_url());
            server.download().unwrap();
        }
        let (cmd, args) = server.command_line(&project);
        eprintln!("{cmd} {args:?}");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut client = crate::lsp::LspClient::new();
        client.start(&cmd, &args, &project).unwrap();

        let start = std::time::Instant::now();
        let deadline = std::time::Duration::from_secs(300);
        while !client.is_connected {
            assert!(start.elapsed() < deadline, "server never initialized");
            client.poll();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        eprintln!("initialized after {:?}", start.elapsed());
        let Some(hover) = var("CU_E2E_HOVER") else {
            return;
        };
        let mut parts = hover.splitn(3, ':');
        let (line, col, expected) = (
            parts.next().unwrap().parse::<u32>().unwrap(),
            parts.next().unwrap().parse::<u32>().unwrap(),
            parts.next().unwrap().to_string(),
        );
        let uri = crate::lsp::client::path_to_uri(&file);
        let text = std::fs::read_to_string(&file).unwrap();
        let lang_id = module
            .manifest
            .capabilities
            .language_ids
            .get(&ext)
            .cloned()
            .unwrap_or(ext);
        client.did_open(&uri, &lang_id, &text);
        // The server may answer empty hovers while it imports the project.
        let mut last = None;
        loop {
            assert!(
                start.elapsed() < deadline,
                "no hover with {expected:?}; last: {last:?}"
            );
            let id = client.request_hover(&uri, line, col);
            let asked = std::time::Instant::now();
            let response = loop {
                if let Some((_, r)) = client.poll().into_iter().find(|(i, _)| *i == id) {
                    break Some(r);
                }
                if asked.elapsed() > std::time::Duration::from_secs(30) {
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            last = response
                .as_ref()
                .and_then(crate::lsp::LspClient::parse_hover);
            if last.as_deref().is_some_and(|h| h.contains(&expected)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        eprintln!("hover after {:?}: {}", start.elapsed(), last.unwrap());
    }

    #[test]
    fn failed_downloads_are_reported_once_and_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["java"]
lsp_server = "srv"
lsp_download = { binary = "srv", urls = { "*" = "http://insecure.invalid/a.zip" } }
"#,
        );
        let s = ModuleServer::for_language([&m], "java").unwrap();
        let mut d = ServerDownloads::default();
        assert!(d.start(&s, None));
        assert!(!d.start(&s, None), "already running");
        let mut done = vec![];
        while d.is_running() {
            done.extend(d.poll());
            std::thread::yield_now();
        }
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].0, "Java");
        assert!(done[0].1.as_ref().unwrap_err().contains("non-https"));
        assert!(!d.start(&s, None), "failed: not retried");
        d.clear_failed();
        assert!(d.start(&s, None));
    }
}
