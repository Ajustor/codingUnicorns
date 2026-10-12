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
            Some(Self {
                module: e.manifest.extension.name.clone(),
                module_id: e.manifest.extension.id.clone(),
                language: language.to_string(),
                dir: e.path.clone(),
                command,
                args,
                download,
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

    /// The downloaded file marking the install, with or without the
    /// executable suffix. `None` without a download, or when the path would
    /// leave the module's folder.
    fn marker_candidates(&self) -> Option<[PathBuf; 2]> {
        let rel = Path::new(&self.download.as_ref()?.binary);
        // The manifest comes from the module: keep the path inside its folder.
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        let plain = rel.components().fold(self.server_dir(), |p, c| p.join(c));
        let mut exe = plain.clone();
        exe.as_mut_os_string().push(std::env::consts::EXE_SUFFIX);
        Some([exe, plain])
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
        let urls = &self.download.as_ref()?.urls;
        urls.get(&crate::extension::remote_registry::platform_key())
            .or_else(|| urls.get("*"))
            .map(String::as_str)
    }

    /// Whether the archive must be fetched before starting the server.
    pub fn needs_download(&self) -> bool {
        self.download_url().is_some()
            && self.installed_file().is_none()
            && (self.needs_files() || find_on_path(&self.command).is_none())
    }

    /// Download and unpack the server's archive (blocking).
    pub fn download(&self) -> Result<(), String> {
        let (Some(url), true) = (self.download_url(), self.marker_candidates().is_some()) else {
            return Err("no download for this platform".into());
        };
        let binary = self.download.as_ref().map_or("", |d| d.binary.as_str());
        crate::extension::archive::download(url, &self.server_dir(), binary)?;
        match self.installed_file() {
            Some(_) => Ok(()),
            None => Err(format!(
                "{} not found in the downloaded archive",
                self.download.as_ref().map_or("", |d| d.binary.as_str())
            )),
        }
    }

    /// Command and arguments to start the server for `workspace`, variables
    /// expanded. Creates the `${dataDir}` folder when it is used.
    pub fn command_line(&self, workspace: &Path) -> (String, Vec<String>) {
        let server_dir = self.server_dir().to_string_lossy().into_owned();
        let data_dir = data_dir(&self.module_id, workspace);
        if self.args.iter().any(|a| a.contains("${dataDir}")) {
            let _ = std::fs::create_dir_all(&data_dir);
        }
        let data_dir = data_dir.to_string_lossy().into_owned();
        let expand = |s: &str| {
            crate::extension::archive::resolve_glob(
                &s.replace("${serverDir}", &server_dir)
                    .replace("${dataDir}", &data_dir)
                    .replace("${os}", os_name()),
            )
        };
        let mut command = with_exe_suffix(expand(&self.command));
        if !self.needs_files() && find_on_path(&command).is_none() {
            if let Some(file) = self.installed_file() {
                command = file.to_string_lossy().into_owned();
            }
        }
        (command, self.args.iter().map(|a| expand(a)).collect())
    }
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
