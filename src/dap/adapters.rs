//! Debug adapters declared by language modules (`[debugger]` in their
//! `manifest.toml`). The IDE itself knows no language: it starts whatever
//! adapter the module describes, downloading it into the module's folder
//! when the module provides an archive and the adapter is not on `PATH`.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::types::DapConfig;
use crate::extension::manifest::DebuggerSpec;
use crate::extension::registry::InstalledExtension;

/// The debug adapter of an installed module.
#[derive(Debug, Clone)]
pub struct DebugAdapter {
    /// Module name, for messages.
    pub module: String,
    pub spec: DebuggerSpec,
    /// Folder of the installed module.
    pub dir: PathBuf,
}

impl DebugAdapter {
    /// The adapter for a VS Code debug `type`, or — without one — for a file
    /// extension, among enabled modules.
    pub fn find<'a>(
        installed: impl IntoIterator<Item = &'a InstalledExtension>,
        adapter_type: Option<&str>,
        file_ext: &str,
    ) -> Option<Self> {
        let adapter_type = adapter_type.filter(|t| !t.is_empty());
        installed
            .into_iter()
            .filter(|e| e.enabled)
            .find(|e| {
                let Some(spec) = &e.manifest.debugger else {
                    return false;
                };
                match adapter_type {
                    Some(t) => spec.types.iter().any(|x| x == t),
                    None => e
                        .manifest
                        .capabilities
                        .languages
                        .iter()
                        .any(|l| l == file_ext),
                }
            })
            .map(|e| Self {
                module: e.manifest.extension.name.clone(),
                spec: e.manifest.debugger.clone().expect("checked above"),
                dir: e.path.clone(),
            })
    }

    /// Whether launch configurations of this debug `type` use this adapter.
    pub fn handles_type(&self, adapter_type: &str) -> bool {
        self.spec.types.iter().any(|t| t == adapter_type)
    }

    /// Name shown in messages (the first command).
    pub fn name(&self) -> &str {
        self.spec
            .command
            .as_slice()
            .first()
            .map(String::as_str)
            .unwrap_or("debugger")
    }

    /// Launch arguments to use when no launch configuration is set.
    pub fn default_launch(&self) -> Option<Value> {
        self.spec
            .default_launch
            .as_ref()
            .and_then(|t| serde_json::to_value(t).ok())
    }

    /// Folder the module's download is unpacked in (`${debuggerDir}`).
    pub fn debugger_dir(&self) -> PathBuf {
        self.dir.join("debugger")
    }

    /// Whether the adapter's arguments use downloaded files.
    fn needs_files(&self) -> bool {
        self.spec.args.iter().any(|a| a.contains("${debuggerDir}"))
    }

    fn command_on_path(&self) -> Option<PathBuf> {
        self.spec
            .command
            .as_slice()
            .iter()
            .find_map(|c| find_on_path(c))
    }

    /// The downloaded file marking the install (`download.binary`), with or
    /// without the executable suffix. `None` when the module has no download
    /// or the path would leave the module's folder.
    fn download_candidates(&self) -> Option<[PathBuf; 2]> {
        let binary = &self.spec.download.as_ref()?.binary;
        let rel = Path::new(binary);
        // The manifest comes from the module: keep the path inside its folder.
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        let plain = rel.components().fold(self.debugger_dir(), |p, c| p.join(c));
        let mut exe = plain.clone();
        exe.as_mut_os_string().push(std::env::consts::EXE_SUFFIX);
        Some([exe, plain])
    }

    /// The downloaded file, when installed.
    fn installed_file(&self) -> Option<PathBuf> {
        self.download_candidates()?
            .into_iter()
            .find(|p| p.is_file())
    }

    /// Whether the download must be fetched before starting: its files are
    /// used by the arguments, or the command is not on `PATH`.
    pub fn needs_download(&self) -> bool {
        self.download_url().is_some()
            && self.installed_file().is_none()
            && (self.needs_files() || self.command_on_path().is_none())
    }

    /// The executable to start: on `PATH`, else the downloaded binary (when
    /// the arguments do not use downloaded files themselves). `None` when the
    /// adapter is not installed.
    pub fn locate(&self) -> Option<PathBuf> {
        if self.needs_files() && self.installed_file().is_none() {
            return None;
        }
        self.command_on_path().or_else(|| {
            if self.needs_files() {
                None
            } else {
                self.installed_file()
            }
        })
    }

    /// Download URL for the running platform.
    pub fn download_url(&self) -> Option<&str> {
        let urls = &self.spec.download.as_ref()?.urls;
        urls.get(&crate::extension::remote_registry::platform_key())
            .map(String::as_str)
    }

    /// Download and unpack the adapter into the module's folder (blocking).
    pub fn download(&self) -> Result<PathBuf, String> {
        let (Some(url), true) = (self.download_url(), self.download_candidates().is_some()) else {
            return Err(format!(
                "the {} module has no {} download for this platform",
                self.module,
                self.name()
            ));
        };
        if !url.starts_with("https://") {
            return Err(format!("refusing non-https download {url}"));
        }
        let bytes = crate::extension::remote_registry::http_fetch(url, 200 * 1024 * 1024)?;
        let dest = self.debugger_dir();
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        if url.ends_with(".zip") {
            extract_zip(&bytes, &dest)?;
        } else {
            extract_tar_gz(&bytes, &dest)?;
        }
        self.installed_file().ok_or_else(|| {
            format!(
                "{} not found in the downloaded archive",
                self.spec
                    .download
                    .as_ref()
                    .map(|d| d.binary.as_str())
                    .unwrap_or("")
            )
        })
    }

    /// Why the adapter cannot be started (not found, nothing to download).
    pub fn missing_message(&self) -> String {
        let mut msg = format!(
            "debugger `{}` (from the {} module) not found",
            self.name(),
            self.module
        );
        if let Some(hint) = &self.spec.install_hint {
            msg.push_str(" — ");
            msg.push_str(hint);
        }
        msg
    }

    /// Session configuration: `${debuggerDir}` expanded in the arguments,
    /// and the launch `type` renamed through the module's `type_map`.
    pub fn config(&self, program: PathBuf, mut launch: Value) -> DapConfig {
        let dir = self.debugger_dir().to_string_lossy().into_owned();
        if let Some(t) = launch["type"].as_str() {
            if let Some(mapped) = self.spec.type_map.get(t) {
                launch["type"] = Value::String(mapped.clone());
            }
        }
        DapConfig {
            adapter_cmd: program.to_string_lossy().into_owned(),
            adapter_args: self
                .spec
                .args
                .iter()
                .map(|a| a.replace("${debuggerDir}", &dir))
                .collect(),
            launch_config: launch,
            transport: self.spec.transport,
        }
    }
}

/// Find an executable on `PATH` (with the platform's executable suffixes).
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&path).find_map(|dir| {
        suffixes
            .iter()
            .map(|s| dir.join(format!("{name}{s}")))
            .find(|p| p.is_file())
    })
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // `enclosed_name` rejects absolute paths and `..` components.
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// `.tar.gz` archives are unpacked with the system `tar`.
fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use crate::process_ext::CommandExt as _;
    let archive = dest.join("download.tar.gz");
    std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .no_window()
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(dest)
        .status();
    let _ = std::fs::remove_file(&archive);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("tar failed ({s})")),
        Err(e) => Err(format!("cannot run tar: {e}")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::extension::manifest::ExtensionManifest;
    use std::io::Write;

    /// An installed module whose manifest has the given `[debugger]` body.
    pub(crate) fn module(
        dir: &Path,
        id: &str,
        languages: &str,
        debugger: &str,
    ) -> InstalledExtension {
        let manifest = ExtensionManifest::parse(&format!(
            r#"[extension]
id = "{id}"
name = "{id}"
version = "1.0.0"
description = ""

[capabilities]
languages = {languages}

{debugger}"#
        ))
        .unwrap();
        InstalledExtension {
            manifest,
            path: dir.join(id),
            lib_path: None,
            enabled: true,
            source: None,
            update_available: None,
        }
    }

    const CSHARP: &str = r#"[debugger]
types = ["coreclr"]
command = "definitely-not-a-real-netcoredbg"
args = ["--interpreter=vscode"]
install_hint = "install it"
[debugger.download]
binary = "netcoredbg/netcoredbg"
[debugger.download.urls]
windows-x86_64 = "https://example.invalid/w.zip"
linux-x86_64 = "https://example.invalid/l.tar.gz"
macos-aarch64 = "https://example.invalid/m.zip"
"#;

    const PYTHON: &str = r#"[debugger]
types = ["python", "debugpy"]
command = ["definitely-not-python-xyz", "definitely-not-python3-xyz"]
args = ["-m", "debugpy.adapter"]
[debugger.default_launch]
program = "${file}"
"#;

    #[test]
    fn find_by_type_then_language_among_enabled_modules() {
        let dir = tempfile::tempdir().unwrap();
        let mut mods = vec![
            module(dir.path(), "cs", r#"["cs"]"#, CSHARP),
            module(dir.path(), "py", r#"["py"]"#, PYTHON),
            module(dir.path(), "md", r#"["md"]"#, ""),
        ];
        let find = |m: &[InstalledExtension], t: Option<&str>, ext: &str| {
            DebugAdapter::find(m, t, ext).map(|a| a.module)
        };
        assert_eq!(find(&mods, Some("coreclr"), "py").as_deref(), Some("cs"));
        assert_eq!(find(&mods, Some("debugpy"), "").as_deref(), Some("py"));
        assert_eq!(find(&mods, None, "cs").as_deref(), Some("cs"));
        assert_eq!(find(&mods, Some(""), "py").as_deref(), Some("py"));
        assert_eq!(
            find(&mods, Some("docker"), "cs"),
            None,
            "explicit type wins"
        );
        assert_eq!(find(&mods, None, "md"), None, "module without debugger");
        mods[0].enabled = false;
        assert_eq!(find(&mods, None, "cs"), None, "disabled module");
    }

    #[test]
    fn spec_accessors() {
        let dir = tempfile::tempdir().unwrap();
        let py = DebugAdapter::find(&[module(dir.path(), "py", r#"["py"]"#, PYTHON)], None, "py")
            .unwrap();
        assert_eq!(py.name(), "definitely-not-python-xyz");
        assert!(py.handles_type("debugpy") && !py.handles_type("coreclr"));
        assert_eq!(py.default_launch().unwrap()["program"], "${file}");
        assert!(py.locate().is_none());
        assert!(py.download_url().is_none());
        let err = py.download().unwrap_err();
        assert!(
            err.contains("no definitely-not-python-xyz download"),
            "{err}"
        );
        assert!(py.missing_message().contains("from the py module"));
        let cfg = py.config(PathBuf::from("python"), serde_json::json!({}));
        assert_eq!(cfg.adapter_args, ["-m", "debugpy.adapter"]);
    }

    #[test]
    fn locate_falls_back_to_the_downloaded_copy() {
        let dir = tempfile::tempdir().unwrap();
        let cs = DebugAdapter::find(&[module(dir.path(), "cs", r#"["cs"]"#, CSHARP)], None, "cs")
            .unwrap();
        assert!(cs.download_url().is_some(), "all CI platforms covered");
        assert!(cs.missing_message().ends_with("— install it"));
        assert!(cs.locate().is_none());
        assert!(cs.needs_download());
        let [exe, _] = cs.download_candidates().unwrap();
        assert!(exe.starts_with(dir.path().join("cs").join("debugger")));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert_eq!(cs.locate(), Some(exe));
        assert!(!cs.needs_download());
    }

    const NODE_TCP: &str = r#"[debugger]
types = ["pwa-node"]
command = "definitely-not-node-xyz"
args = ["${debuggerDir}/js-debug/src/dapDebugServer.js", "${port}"]
transport = "tcp"
type_map = { node = "pwa-node" }
[debugger.download]
binary = "js-debug/src/dapDebugServer.js"
[debugger.download.urls]
windows-x86_64 = "https://example.invalid/j.tgz"
linux-x86_64 = "https://example.invalid/j.tgz"
macos-aarch64 = "https://example.invalid/j.tgz"
"#;

    #[test]
    fn downloaded_files_used_by_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let js = DebugAdapter::find(
            &[module(dir.path(), "js", r#"["js"]"#, NODE_TCP)],
            None,
            "js",
        )
        .unwrap();
        assert!(js.needs_download(), "arguments need the files");
        // The script is installed (no .exe) but the runtime is missing.
        let [_, script] = js.download_candidates().unwrap();
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "").unwrap();
        assert!(!js.needs_download());
        assert!(js.locate().is_none(), "the script is not the command");

        let cfg = js.config(PathBuf::from("node"), serde_json::json!({"type": "node"}));
        assert_eq!(cfg.launch_config["type"], "pwa-node", "type_map applied");
        assert_eq!(
            cfg.transport,
            crate::extension::manifest::DebuggerTransport::Tcp
        );
        assert!(cfg.adapter_args[0].starts_with(&*js.debugger_dir().to_string_lossy()));
        assert_eq!(
            cfg.adapter_args[1], "${port}",
            "port is set by the transport"
        );
    }

    #[test]
    fn binary_path_cannot_leave_the_module_folder() {
        let dir = tempfile::tempdir().unwrap();
        let evil = CSHARP.replace("netcoredbg/netcoredbg", "../../evil");
        let cs = DebugAdapter::find(&[module(dir.path(), "cs", r#"["cs"]"#, &evil)], None, "cs")
            .unwrap();
        assert!(cs.download_candidates().is_none());
        assert!(cs.download().is_err());
    }

    #[test]
    fn zip_extraction_keeps_folders_and_skips_escaping_paths() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            w.add_directory("netcoredbg/", opts).unwrap();
            w.start_file("netcoredbg/netcoredbg.exe", opts).unwrap();
            w.write_all(b"bin").unwrap();
            w.start_file("../evil.txt", opts).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("tools");
        extract_zip(buf.get_ref(), &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("netcoredbg").join("netcoredbg.exe")).unwrap(),
            b"bin"
        );
        assert!(!dir.path().join("evil.txt").exists());
    }

    #[test]
    fn find_on_path_misses_unknown_programs() {
        assert!(find_on_path("definitely-not-a-real-program-xyz").is_none());
    }
}
