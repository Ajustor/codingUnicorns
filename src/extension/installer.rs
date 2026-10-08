use std::path::PathBuf;
use std::sync::mpsc;

use super::manifest::{ExtensionSource, SourceKind};
use crate::process_ext::CommandExt as _;

/// Destination directory for an extension. Re-validates the id (manifests are
/// already validated by `ExtensionManifest::parse`, but the fields are public)
/// so no install path can ever join an unsafe id onto `extensions_dir`.
fn extension_dest_dir(
    extensions_dir: &std::path::Path,
    manifest: &super::manifest::ExtensionManifest,
) -> anyhow::Result<PathBuf> {
    super::manifest::validate_extension_id(&manifest.extension.id)?;
    Ok(extensions_dir.join(&manifest.extension.id))
}

fn write_source(dest_dir: &std::path::Path, source: &ExtensionSource) {
    if let Ok(toml_str) = toml::to_string(source) {
        let _ = std::fs::write(dest_dir.join("source.toml"), toml_str);
    }
}

// ── Module discovery ─────────────────────────────────────────────────────────

/// Info about a workspace member that has a manifest.toml.
#[derive(Debug, Clone)]
pub struct DiscoveredModule {
    /// Cargo workspace member name (directory name).
    pub member: String,
    /// Parsed manifest info.
    pub manifest: super::manifest::ExtensionManifest,
}

/// Discover installable modules in a workspace directory.
/// Returns all members that have a `manifest.toml`.
pub fn discover_modules(workspace_path: &std::path::Path) -> anyhow::Result<Vec<DiscoveredModule>> {
    let cargo_toml = std::fs::read_to_string(workspace_path.join("Cargo.toml"))?;
    let members = parse_workspace_members(&cargo_toml)?;
    let mut result = Vec::new();
    for member in members {
        let manifest_path = workspace_path.join(&member).join("manifest.toml");
        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = super::manifest::ExtensionManifest::parse(&content) {
                result.push(DiscoveredModule { member, manifest });
            }
        }
    }
    Ok(result)
}

// ── Workspace installer ───────────────────────────────────────────────────────

/// Status events emitted by `install_from_workspace` / `install_group_from_git`.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceStatus {
    Idle,
    /// Cloning a git repository.
    Cloning,
    /// Running `cargo build --release` on the whole workspace.
    Building,
    /// Copying one module into the extensions directory.
    Installing {
        current: String,
        done: usize,
        total: usize,
    },
    /// Installing an external dependency for a module.
    InstallingDep {
        module: String,
        step: String,
    },
    /// One module failed (non-fatal — install continues for the rest).
    ModuleFailed {
        name: String,
        reason: String,
    },
    /// All done — `installed` out of `total` modules were installed.
    Done {
        installed: usize,
        total: usize,
    },
    /// Fatal error (workspace-level).
    Failed(String),
}

/// Build selected members of a Cargo workspace that have a `manifest.toml` and
/// install them into `extensions_dir`.
///
/// When `selected_members` is `None`, all modules with manifest.toml are installed.
/// When `Some(list)`, only matching member directory names are installed.
///
/// Progress is streamed via the returned channel so the UI can update live.
pub fn install_from_workspace(
    workspace_path: PathBuf,
    extensions_dir: PathBuf,
    selected_members: Option<Vec<String>>,
) -> mpsc::Receiver<WorkspaceStatus> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        workspace_install_inner(
            workspace_path,
            extensions_dir,
            selected_members.as_deref(),
            &tx,
        );
    });
    rx
}

/// Clone a git repository and install all workspace members that have a
/// `manifest.toml`. If the repo is a single extension (no `[workspace]`
/// in Cargo.toml), it is installed as a single module.
pub fn install_group_from_git(
    repo_url: String,
    extensions_dir: PathBuf,
) -> mpsc::Receiver<WorkspaceStatus> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // 1. Clone
        let _ = tx.send(WorkspaceStatus::Cloning);
        let tmp_dir = match tempdir_for_clone(&repo_url) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::Failed(format!("Clone failed: {e}")));
                return;
            }
        };
        if let Err(e) = git2::Repository::clone(&repo_url, &tmp_dir) {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Clone failed: {e}")));
            return;
        }

        // 2. Read Cargo.toml and detect workspace vs single extension
        let cargo_toml_path = tmp_dir.join("Cargo.toml");
        let cargo_toml_str = match std::fs::read_to_string(&cargo_toml_path) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::Failed(format!("No Cargo.toml: {e}")));
                return;
            }
        };
        let value: toml::Value = match toml::from_str(&cargo_toml_str) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::Failed(format!("Invalid Cargo.toml: {e}")));
                return;
            }
        };

        if value.get("workspace").is_some() {
            // Multi-module workspace
            workspace_install_inner(tmp_dir, extensions_dir, None, &tx);
        } else {
            // Single extension — install it and report as 1/1
            single_git_install_inner(&tmp_dir, &extensions_dir, &repo_url, &tx);
        }
    });
    rx
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Core workspace install logic shared by `install_from_workspace` and
/// `install_group_from_git`.
fn workspace_install_inner(
    workspace_path: PathBuf,
    extensions_dir: PathBuf,
    selected_members: Option<&[String]>,
    tx: &mpsc::Sender<WorkspaceStatus>,
) {
    // 1. Parse workspace Cargo.toml
    let cargo_toml_path = workspace_path.join("Cargo.toml");
    let cargo_toml_str = match std::fs::read_to_string(&cargo_toml_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!(
                "Cannot read Cargo.toml: {e}"
            )));
            return;
        }
    };
    let members = match parse_workspace_members(&cargo_toml_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Invalid workspace: {e}")));
            return;
        }
    };

    // 2. Keep only members that have a manifest.toml and match the selection
    let modules: Vec<String> = members
        .into_iter()
        .filter(|m| workspace_path.join(m).join("manifest.toml").exists())
        .filter(|m| {
            selected_members
                .map(|sel| sel.iter().any(|s| s == m))
                .unwrap_or(true)
        })
        .collect();

    if modules.is_empty() {
        let _ = tx.send(WorkspaceStatus::Failed(
            "No modules with manifest.toml found in this workspace.".to_string(),
        ));
        return;
    }

    // 3. Build the whole workspace once
    let _ = tx.send(WorkspaceStatus::Building);
    let build_out = std::process::Command::new("cargo")
        .no_window()
        .args(["build", "--release"])
        .current_dir(&workspace_path)
        .output();

    match build_out {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let err: String = String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(400)
                .collect();
            let _ = tx.send(WorkspaceStatus::Failed(format!("Build failed:\n{err}")));
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Cannot run cargo: {e}")));
            return;
        }
    }

    // 4. Install each module
    let total = modules.len();
    let release_dir = workspace_path.join("target").join("release");
    let mut installed = 0;

    for (i, member) in modules.iter().enumerate() {
        let _ = tx.send(WorkspaceStatus::Installing {
            current: member.clone(),
            done: i,
            total,
        });

        let member_path = workspace_path.join(member);
        let manifest_path = member_path.join("manifest.toml");

        // Read manifest
        let manifest_str = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::ModuleFailed {
                    name: member.clone(),
                    reason: format!("manifest.toml unreadable: {e}"),
                });
                continue;
            }
        };
        let manifest = match super::manifest::ExtensionManifest::parse(&manifest_str) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::ModuleFailed {
                    name: member.clone(),
                    reason: format!("Invalid manifest: {e}"),
                });
                continue;
            }
        };

        // Find the compiled library in the shared workspace target/release/
        let lib_name = member.replace('-', "_");
        let lib_path = match find_lib_in_release_dir(&release_dir, &lib_name) {
            Some(p) => p,
            None => {
                let _ = tx.send(WorkspaceStatus::ModuleFailed {
                    name: member.clone(),
                    reason: format!("lib{lib_name}.so not found in target/release"),
                });
                continue;
            }
        };

        // Copy lib + manifest to extensions dir
        let dest_dir = match extension_dest_dir(&extensions_dir, &manifest) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(WorkspaceStatus::ModuleFailed {
                    name: member.clone(),
                    reason: format!("Invalid manifest: {e}"),
                });
                continue;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            let _ = tx.send(WorkspaceStatus::ModuleFailed {
                name: member.clone(),
                reason: format!("mkdir: {e}"),
            });
            continue;
        }
        if let Err(e) = copy_lib_safe(
            &lib_path,
            &dest_dir.join(lib_path.file_name().unwrap_or_default()),
        ) {
            let _ = tx.send(WorkspaceStatus::ModuleFailed {
                name: member.clone(),
                reason: format!("Copy library: {e}"),
            });
            continue;
        }
        if let Err(e) = std::fs::copy(&manifest_path, dest_dir.join("manifest.toml")) {
            let _ = tx.send(WorkspaceStatus::ModuleFailed {
                name: member.clone(),
                reason: format!("Copy manifest: {e}"),
            });
            continue;
        }
        write_source(
            &dest_dir,
            &ExtensionSource {
                kind: SourceKind::Workspace,
                path: Some(workspace_path.to_string_lossy().to_string()),
                member: Some(member.clone()),
                url: None,
                id: None,
            },
        );

        // Install external dependencies declared in the manifest.
        let member_name = member.clone();
        let dep_errors = install_deps(&manifest.dependencies, |step| {
            let _ = tx.send(WorkspaceStatus::InstallingDep {
                module: member_name.clone(),
                step,
            });
        });
        for err in dep_errors {
            let _ = tx.send(WorkspaceStatus::ModuleFailed {
                name: member.clone(),
                reason: format!("dependency error: {err}"),
            });
        }

        installed += 1;
    }

    let _ = tx.send(WorkspaceStatus::Done { installed, total });
}

/// Install a single-extension git clone. Reports as 1-of-1 using
/// `WorkspaceStatus` so the git-group UI can display it uniformly.
fn single_git_install_inner(
    dir: &std::path::Path,
    extensions_dir: &std::path::Path,
    repo_url: &str,
    tx: &mpsc::Sender<WorkspaceStatus>,
) {
    let _ = tx.send(WorkspaceStatus::Building);

    let manifest_path = dir.join("manifest.toml");
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("No manifest.toml: {e}")));
            return;
        }
    };
    let manifest = match super::manifest::ExtensionManifest::parse(&manifest_str) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Invalid manifest: {e}")));
            return;
        }
    };

    let build_result = std::process::Command::new("cargo")
        .no_window()
        .args(["build", "--release"])
        .current_dir(dir)
        .output();
    match build_result {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let err: String = String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(400)
                .collect();
            let _ = tx.send(WorkspaceStatus::Failed(format!("Build failed:\n{err}")));
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Cannot run cargo: {e}")));
            return;
        }
    }

    let name = manifest.extension.name.clone();
    let _ = tx.send(WorkspaceStatus::Installing {
        current: name.clone(),
        done: 0,
        total: 1,
    });

    let dest = match extension_dest_dir(extensions_dir, &manifest) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Invalid manifest: {e}")));
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dest) {
        let _ = tx.send(WorkspaceStatus::Failed(format!("mkdir: {e}")));
        return;
    }
    if let Err(e) = std::fs::copy(&manifest_path, dest.join("manifest.toml")) {
        let _ = tx.send(WorkspaceStatus::Failed(format!("Copy manifest: {e}")));
        return;
    }
    let release_dir = dir.join("target").join("release");
    if let Some(lib_file) = find_lib_file(&release_dir) {
        if let Err(e) = copy_lib_safe(
            &lib_file,
            &dest.join(lib_file.file_name().unwrap_or_default()),
        ) {
            let _ = tx.send(WorkspaceStatus::Failed(format!("Copy lib: {e}")));
            return;
        }
    }
    write_source(
        &dest,
        &ExtensionSource {
            kind: SourceKind::Git,
            url: Some(repo_url.to_string()),
            id: None,
            path: None,
            member: None,
        },
    );
    let module_name = name.clone();
    install_deps(&manifest.dependencies, |step| {
        let _ = tx.send(WorkspaceStatus::InstallingDep {
            module: module_name.clone(),
            step,
        });
    });
    let _ = tx.send(WorkspaceStatus::Done {
        installed: 1,
        total: 1,
    });
}

/// Parse `[workspace].members` from a Cargo.toml string.
fn parse_workspace_members(cargo_toml: &str) -> anyhow::Result<Vec<String>> {
    let value: toml::Value = toml::from_str(cargo_toml)?;
    let members = value
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow::anyhow!("No [workspace].members found"))?;
    Ok(members
        .iter()
        .filter_map(|m| m.as_str().map(|s| s.to_string()))
        .collect())
}

/// Find `lib{name}.so` / `.dylib` / `.dll` in a release directory.
fn find_lib_in_release_dir(release_dir: &std::path::Path, lib_name: &str) -> Option<PathBuf> {
    let candidates = [
        format!("lib{lib_name}.so"),
        format!("lib{lib_name}.dylib"),
        format!("{lib_name}.dll"),
    ];
    for name in &candidates {
        let path = release_dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstallStatus {
    Idle,
    Cloning,
    Building,
    Installing,
    /// Installing an external dependency (e.g. "npm install -g pylsp").
    InstallingDep(String),
    Done,
    Failed(String),
}

pub struct InstallJob {
    pub repo_url: String,
    pub status: InstallStatus,
    pub log: Vec<String>,
}

impl InstallJob {
    pub fn new(repo_url: String) -> Self {
        Self {
            repo_url,
            status: InstallStatus::Idle,
            log: Vec::new(),
        }
    }

    /// Run installation in a background thread, sending status updates via a channel.
    pub fn start(repo_url: String, extensions_dir: PathBuf) -> mpsc::Receiver<InstallStatus> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // 1. Clone
            let _ = tx.send(InstallStatus::Cloning);
            let tmp_dir = match tempdir_for_clone(&repo_url) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Clone failed: {e}")));
                    return;
                }
            };
            match git2::Repository::clone(&repo_url, &tmp_dir) {
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Clone failed: {e}")));
                    return;
                }
            }

            // 2. Read manifest
            let manifest_path = tmp_dir.join("manifest.toml");
            let manifest_str = match std::fs::read_to_string(&manifest_path) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("No manifest.toml: {e}")));
                    return;
                }
            };
            let manifest = match super::manifest::ExtensionManifest::parse(&manifest_str) {
                Ok(m) => m,
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Invalid manifest: {e}")));
                    return;
                }
            };

            // 3. Build
            let _ = tx.send(InstallStatus::Building);
            let build_result = std::process::Command::new("cargo")
                .no_window()
                .args(["build", "--release"])
                .current_dir(&tmp_dir)
                .output();
            match build_result {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let _ = tx.send(InstallStatus::Failed(format!("Build failed: {stderr}")));
                    return;
                }
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Build error: {e}")));
                    return;
                }
            }

            // 4. Copy artifact + manifest
            let _ = tx.send(InstallStatus::Installing);
            let dest = match extension_dest_dir(&extensions_dir, &manifest) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Invalid manifest: {e}")));
                    return;
                }
            };
            if let Err(e) = std::fs::create_dir_all(&dest) {
                let _ = tx.send(InstallStatus::Failed(format!("mkdir failed: {e}")));
                return;
            }

            // Copy manifest
            if let Err(e) = std::fs::copy(&manifest_path, dest.join("manifest.toml")) {
                let _ = tx.send(InstallStatus::Failed(format!("Copy manifest failed: {e}")));
                return;
            }

            // Find and copy .so/.dll
            let release_dir = tmp_dir.join("target").join("release");
            if let Some(lib_file) = find_lib_file(&release_dir) {
                let dest_lib = dest.join(lib_file.file_name().unwrap_or_default());
                if let Err(e) = copy_lib_safe(&lib_file, &dest_lib) {
                    let _ = tx.send(InstallStatus::Failed(format!("Copy lib failed: {e}")));
                    return;
                }
            }

            write_source(
                &dest,
                &ExtensionSource {
                    kind: SourceKind::Git,
                    url: Some(repo_url.clone()),
                    id: None,
                    path: None,
                    member: None,
                },
            );

            // 5. Install external dependencies.
            install_deps(&manifest.dependencies, |step| {
                let _ = tx.send(InstallStatus::InstallingDep(step));
            });

            let _ = tx.send(InstallStatus::Done);
        });
        rx
    }
}

/// Install an extension from a local directory.
/// The directory must contain a `manifest.toml`.
/// If it contains a `target/release/lib*.so` (or `.dll` / `.dylib`), use it directly.
/// Otherwise, try to build with `cargo build --release` first.
pub fn install_from_folder(
    folder: PathBuf,
    extensions_dir: PathBuf,
) -> mpsc::Receiver<InstallStatus> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // 1. Read manifest.toml
        let manifest_path = folder.join("manifest.toml");
        if !manifest_path.exists() {
            let _ = tx.send(InstallStatus::Failed(
                "No manifest.toml found in folder".to_string(),
            ));
            return;
        }

        let manifest_str = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(InstallStatus::Failed(format!("Cannot read manifest: {e}")));
                return;
            }
        };

        let manifest = match super::manifest::ExtensionManifest::parse(&manifest_str) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(InstallStatus::Failed(format!("Invalid manifest.toml: {e}")));
                return;
            }
        };

        let dest_dir = match extension_dest_dir(&extensions_dir, &manifest) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(InstallStatus::Failed(format!("Invalid manifest.toml: {e}")));
                return;
            }
        };

        // 2. Look for pre-built library
        let lib_name = folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("extension")
            .replace('-', "_");

        let lib_path = if let Some(p) = find_prebuilt_lib(&folder, &lib_name) {
            p
        } else {
            // 3. Build with cargo
            let _ = tx.send(InstallStatus::Building);
            match std::process::Command::new("cargo")
                .no_window()
                .args(["build", "--release"])
                .current_dir(&folder)
                .output()
            {
                Ok(out) if out.status.success() => match find_prebuilt_lib(&folder, &lib_name) {
                    Some(p) => p,
                    None => {
                        let _ = tx.send(InstallStatus::Failed(
                            "Build succeeded but no library found".to_string(),
                        ));
                        return;
                    }
                },
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).to_string();
                    let truncated: String = err.chars().take(200).collect();
                    let _ = tx.send(InstallStatus::Failed(format!("Build failed: {truncated}")));
                    return;
                }
                Err(e) => {
                    let _ = tx.send(InstallStatus::Failed(format!("Cannot run cargo: {e}")));
                    return;
                }
            }
        };

        // 4. Copy to extensions dir
        let _ = tx.send(InstallStatus::Installing);
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            let _ = tx.send(InstallStatus::Failed(format!("Cannot create dir: {e}")));
            return;
        }

        let dest_lib = dest_dir.join(lib_path.file_name().unwrap_or_default());
        if let Err(e) = copy_lib_safe(&lib_path, &dest_lib) {
            let _ = tx.send(InstallStatus::Failed(format!("Cannot copy library: {e}")));
            return;
        }
        if let Err(e) = std::fs::copy(&manifest_path, dest_dir.join("manifest.toml")) {
            let _ = tx.send(InstallStatus::Failed(format!("Cannot copy manifest: {e}")));
            return;
        }
        write_source(
            &dest_dir,
            &ExtensionSource {
                kind: SourceKind::Folder,
                path: Some(folder.to_string_lossy().to_string()),
                member: None,
                url: None,
                id: None,
            },
        );

        // Install external dependencies.
        install_deps(&manifest.dependencies, |step| {
            let _ = tx.send(InstallStatus::InstallingDep(step));
        });

        let _ = tx.send(InstallStatus::Done);
    });
    rx
}

fn find_prebuilt_lib(folder: &std::path::Path, lib_name: &str) -> Option<PathBuf> {
    let release_dir = folder.join("target").join("release");

    let candidates = [
        format!("lib{lib_name}.so"),    // Linux
        format!("lib{lib_name}.dylib"), // macOS
        format!("{lib_name}.dll"),      // Windows
    ];

    for name in &candidates {
        let path = release_dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn tempdir_for_clone(repo_url: &str) -> anyhow::Result<PathBuf> {
    let name = repo_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("extension")
        .trim_end_matches(".git")
        .to_string();
    let tmp = std::env::temp_dir().join(format!("wu-ext-{name}-{}", uuid::Uuid::new_v4()));
    Ok(tmp)
}

/// Install all external dependencies declared in a module's manifest.
/// Calls `progress(step_description)` before each command.
/// Returns a list of error strings (non-fatal — the caller decides what to do).
/// Create a Command that works on both Unix and Windows.
/// On Windows, scripts like `npm`, `pip3`, `go` are batch files (.cmd/.bat)
/// and need to be invoked through `cmd /C`.
fn shell_command(program: &str) -> std::process::Command {
    if cfg!(target_os = "windows") {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", program]).no_window();
        cmd
    } else {
        std::process::Command::new(program)
    }
}

fn install_deps(
    deps: &super::manifest::Dependencies,
    mut progress: impl FnMut(String),
) -> Vec<String> {
    let mut errors = Vec::new();

    // npm packages
    if !deps.npm.is_empty() {
        let step = format!("npm install -g {}", deps.npm.join(" "));
        progress(step.clone());
        let mut cmd = shell_command("npm");
        cmd.arg("install").arg("-g");
        for pkg in &deps.npm {
            cmd.arg(pkg);
        }
        match cmd.output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                errors.push(format!(
                    "npm: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                ));
            }
            Err(e) => errors.push(format!("npm not found: {e}")),
        }
    }

    // pip packages
    if !deps.pip.is_empty() {
        for pkg in &deps.pip {
            let step = format!("pip3 install {pkg}");
            progress(step.clone());
            match shell_command("pip3").args(["install", pkg]).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(format!(
                    "pip3 {pkg}: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                Err(e) => errors.push(format!("pip3 not found: {e}")),
            }
        }
    }

    // cargo packages
    if !deps.cargo.is_empty() {
        for pkg in &deps.cargo {
            let step = format!("cargo install {pkg}");
            progress(step.clone());
            match shell_command("cargo").args(["install", pkg]).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(format!(
                    "cargo install {pkg}: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                Err(e) => errors.push(format!("cargo not found: {e}")),
            }
        }
    }

    // go packages
    if !deps.go.is_empty() {
        for pkg in &deps.go {
            let step = format!("go install {pkg}");
            progress(step.clone());
            match shell_command("go")
                .args(["install", pkg])
                .env("GOPATH", {
                    // Prefer $GOPATH, fall back to ~/go
                    std::env::var("GOPATH").unwrap_or_else(|_| {
                        dirs_next::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join("go")
                            .to_string_lossy()
                            .to_string()
                    })
                })
                .output()
            {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(format!(
                    "go install {pkg}: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                Err(e) => errors.push(format!("go not found: {e}")),
            }
        }
    }

    // .NET global tools (`dotnet tool update --global <pkg>` — installs if missing,
    // and is idempotent unlike `install`, so re-installing a module never errors).
    if !deps.dotnet.is_empty() {
        for pkg in &deps.dotnet {
            let (name, version) = split_pkg_version(pkg);
            let step = match version {
                Some(v) => format!("dotnet tool update --global {name} --version {v}"),
                None => format!("dotnet tool update --global {name}"),
            };
            progress(step);
            let mut cmd = shell_command("dotnet");
            cmd.args(["tool", "update", "--global", name]);
            if let Some(v) = version {
                cmd.args(["--version", v]);
            }
            match cmd.output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(format!(
                    "dotnet tool {name}: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                Err(e) => errors.push(format!("dotnet not found: {e}")),
            }
        }
    }

    errors
}

/// Split a dependency spec of the form `name@version` into `(name, Some(version))`,
/// or `(name, None)` when no version is pinned.
fn split_pkg_version(pkg: &str) -> (&str, Option<&str>) {
    match pkg.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (pkg, None),
    }
}

/// Human-readable list of the external dependencies a module installed, e.g.
/// `["csharp-ls (dotnet)", "python-lsp-server (pip)"]`. Used by the uninstall
/// dialog to tell the user what would be removed. Empty when the module has none.
pub fn dependency_summary(deps: &super::manifest::Dependencies) -> Vec<String> {
    let mut out = Vec::new();
    for p in &deps.npm {
        out.push(format!("{} (npm)", split_pkg_version(p).0));
    }
    for p in &deps.pip {
        out.push(format!("{} (pip)", split_pkg_version(p).0));
    }
    for p in &deps.cargo {
        out.push(format!("{} (cargo)", split_pkg_version(p).0));
    }
    for p in &deps.go {
        out.push(format!("{} (go)", p));
    }
    for p in &deps.dotnet {
        out.push(format!("{} (dotnet tool)", split_pkg_version(p).0));
    }
    out
}

/// Remove the external dependencies a module installed. Best-effort and
/// idempotent — missing packages are not treated as errors. Returns a list of
/// human-readable error strings for failures the caller may want to surface.
///
/// `go` tools are skipped: `go install` leaves a bare binary in `GOBIN` with no
/// reliable uninstall command, so removing it would require guessing the path.
pub fn uninstall_deps(deps: &super::manifest::Dependencies) -> Vec<String> {
    let mut errors = Vec::new();

    for pkg in &deps.npm {
        let name = split_pkg_version(pkg).0;
        run_uninstall(&mut errors, "npm", "npm", &["uninstall", "-g", name]);
    }
    for pkg in &deps.pip {
        let name = split_pkg_version(pkg).0;
        run_uninstall(&mut errors, "pip3", "pip3", &["uninstall", "-y", name]);
    }
    for pkg in &deps.cargo {
        let name = split_pkg_version(pkg).0;
        run_uninstall(&mut errors, "cargo", "cargo", &["uninstall", name]);
    }
    for pkg in &deps.dotnet {
        let name = split_pkg_version(pkg).0;
        run_uninstall(
            &mut errors,
            "dotnet",
            "dotnet",
            &["tool", "uninstall", "--global", name],
        );
    }

    errors
}

/// Run a single uninstall command, recording a short error on failure.
fn run_uninstall(errors: &mut Vec<String>, label: &str, program: &str, args: &[&str]) {
    match shell_command(program).args(args).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => errors.push(format!(
            "{label}: {}",
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        )),
        Err(e) => errors.push(format!("{label} not found: {e}")),
    }
}

/// Copy a library file to the destination, handling the case where the destination
/// is locked by a running process (common on Windows when a DLL is loaded).
/// Strategy: rename the old file out of the way first, then copy the new one.
fn copy_lib_safe(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    if dest.exists() {
        // Try renaming the old file to .old — Windows allows renaming loaded DLLs
        let old_path = dest.with_extension("old");
        // Remove any previous .old file
        let _ = std::fs::remove_file(&old_path);
        // Rename current DLL to .old (works even if loaded on Windows)
        if std::fs::rename(dest, &old_path).is_err() {
            // If rename fails too, try removing directly (will fail if locked)
            let _ = std::fs::remove_file(dest);
        }
    }
    std::fs::copy(src, dest)?;
    // Clean up the .old file if possible
    let old_path = dest.with_extension("old");
    let _ = std::fs::remove_file(&old_path);
    Ok(())
}

fn find_lib_file(release_dir: &std::path::Path) -> Option<PathBuf> {
    super::registry::find_platform_lib(release_dir)
}

// ── ZIP installer ────────────────────────────────────────────────────────────

/// A module discovered inside a ZIP archive.
#[derive(Debug, Clone)]
pub struct ZipModule {
    /// Top-level directory name inside the ZIP (e.g. "rust-lang").
    pub dir_name: String,
    pub manifest: super::manifest::ExtensionManifest,
}

/// Scan a ZIP file for installable modules (directories with manifest.toml).
pub fn discover_zip_modules(zip_path: &std::path::Path) -> anyhow::Result<Vec<ZipModule>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut manifests = Vec::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        // Look for manifest.toml at depth 1: "dir_name/manifest.toml"
        if name.ends_with("/manifest.toml") || name.ends_with("\\manifest.toml") {
            let parts: Vec<&str> = name.split(['/', '\\']).collect();
            if parts.len() == 2 {
                let dir_name = parts[0].to_string();
                let content = std::io::read_to_string(entry)?;
                if let Ok(manifest) = super::manifest::ExtensionManifest::parse(&content) {
                    manifests.push(ZipModule { dir_name, manifest });
                }
            }
        }
    }
    Ok(manifests)
}

/// Install selected modules from a ZIP archive into `extensions_dir`.
pub fn install_from_zip(
    zip_path: PathBuf,
    extensions_dir: PathBuf,
    selected_dirs: Option<Vec<String>>,
) -> mpsc::Receiver<WorkspaceStatus> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        zip_install_inner(zip_path, extensions_dir, selected_dirs.as_deref(), &tx);
    });
    rx
}

fn zip_install_inner(
    zip_path: PathBuf,
    extensions_dir: PathBuf,
    selected_dirs: Option<&[String]>,
    tx: &mpsc::Sender<WorkspaceStatus>,
) {
    let source_path = zip_path.to_string_lossy().to_string();
    zip_install_core(
        &zip_path,
        &extensions_dir,
        selected_dirs,
        &ZipInstallOptions {
            source: &|dir_name| ExtensionSource {
                kind: SourceKind::Zip,
                path: Some(source_path.clone()),
                member: Some(dir_name.to_string()),
                url: None,
                id: None,
            },
            install_deps: false,
        },
        &|status| {
            let _ = tx.send(status);
        },
    );
}

/// How `zip_install_core` records and finishes each installed module.
pub(crate) struct ZipInstallOptions<'a> {
    /// `source.toml` content for a module, given its directory inside the ZIP.
    pub source: &'a dyn Fn(&str) -> ExtensionSource,
    /// Run `install_deps` for the module's declared external dependencies.
    pub install_deps: bool,
}

/// Extract the selected modules of a ZIP into `extensions_dir`, reporting
/// progress through `emit`. Shared by the plain ZIP and the registry installer.
pub(crate) fn zip_install_core(
    zip_path: &std::path::Path,
    extensions_dir: &std::path::Path,
    selected_dirs: Option<&[String]>,
    opts: &ZipInstallOptions,
    emit: &dyn Fn(WorkspaceStatus),
) {
    let file = match std::fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            emit(WorkspaceStatus::Failed(format!("Cannot open ZIP: {e}")));
            return;
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            emit(WorkspaceStatus::Failed(format!("Invalid ZIP: {e}")));
            return;
        }
    };

    // Discover modules in the ZIP
    let mut modules: Vec<(String, super::manifest::ExtensionManifest)> = Vec::new();
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let name = entry.name().to_string();
        if name.ends_with("/manifest.toml") || name.ends_with("\\manifest.toml") {
            let parts: Vec<&str> = name.split(['/', '\\']).collect();
            if parts.len() == 2 {
                let dir_name = parts[0].to_string();
                if let Ok(content) = std::io::read_to_string(entry) {
                    if let Ok(manifest) = super::manifest::ExtensionManifest::parse(&content) {
                        if selected_dirs
                            .map(|sel| sel.iter().any(|s| s == &dir_name))
                            .unwrap_or(true)
                        {
                            modules.push((dir_name, manifest));
                        }
                    }
                }
            }
        }
    }

    if modules.is_empty() {
        emit(WorkspaceStatus::Failed(
            "No modules with manifest.toml found in this ZIP.".to_string(),
        ));
        return;
    }

    let total = modules.len();
    let mut installed = 0;

    for (i, (dir_name, manifest)) in modules.iter().enumerate() {
        emit(WorkspaceStatus::Installing {
            current: manifest.extension.name.clone(),
            done: i,
            total,
        });

        let dest_dir = match extension_dest_dir(extensions_dir, manifest) {
            Ok(d) => d,
            Err(e) => {
                emit(WorkspaceStatus::ModuleFailed {
                    name: dir_name.clone(),
                    reason: format!("Invalid manifest: {e}"),
                });
                continue;
            }
        };
        let _ = std::fs::create_dir_all(&dest_dir);

        // Extract all files from this module's directory. Entry names are
        // normalised to `/` separators (some Windows tools write `\`), the
        // same way discovery splits them.
        let prefix = format!("{dir_name}/");
        let mut has_lib = false;
        let mut has_manifest = false;
        let mut lib_failed = false;

        // Re-open archive for extraction (ZipArchive doesn't support seeking back)
        let Ok(file2) = std::fs::File::open(zip_path) else {
            emit(WorkspaceStatus::ModuleFailed {
                name: dir_name.clone(),
                reason: "Cannot re-open ZIP".to_string(),
            });
            continue;
        };
        let Ok(mut archive2) = zip::ZipArchive::new(file2) else {
            continue;
        };

        for j in 0..archive2.len() {
            let Ok(mut entry) = archive2.by_index(j) else {
                continue;
            };
            let entry_name = entry.name().replace('\\', "/");
            if !entry_name.starts_with(&prefix) {
                continue;
            }
            let rel_path = &entry_name[prefix.len()..];
            if rel_path.is_empty() || entry.is_dir() || entry_name.ends_with('/') {
                continue;
            }

            // Only extract known file types
            let file_name = std::path::Path::new(rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Archives may bundle every platform's build; keep only ours.
            let is_lib = super::registry::is_platform_lib(std::path::Path::new(file_name));
            let is_manifest = file_name == "manifest.toml";

            if !is_lib && !is_manifest {
                continue;
            }

            let dest_file = dest_dir.join(file_name);
            if is_lib {
                // Use safe copy for DLLs (handles locked files on Windows)
                let tmp_path = dest_dir.join(format!("{file_name}.tmp"));
                if let Ok(mut out) = std::fs::File::create(&tmp_path) {
                    let _ = std::io::copy(&mut entry, &mut out);
                }
                if let Err(e) = copy_lib_safe(&tmp_path, &dest_file) {
                    emit(WorkspaceStatus::ModuleFailed {
                        name: dir_name.clone(),
                        reason: format!("Copy lib: {e}"),
                    });
                    lib_failed = true;
                }
                let _ = std::fs::remove_file(&tmp_path);
                has_lib = true;
            } else {
                if let Ok(mut out) = std::fs::File::create(&dest_file) {
                    let _ = std::io::copy(&mut entry, &mut out);
                }
                has_manifest = true;
            }
        }

        if lib_failed {
            // Already reported; the module is not usable.
        } else if has_lib && has_manifest {
            write_source(&dest_dir, &(opts.source)(dir_name));
            if opts.install_deps {
                for err in install_deps(&manifest.dependencies, |step| {
                    emit(WorkspaceStatus::InstallingDep {
                        module: dir_name.clone(),
                        step,
                    });
                }) {
                    emit(WorkspaceStatus::ModuleFailed {
                        name: dir_name.clone(),
                        reason: format!("dependency error: {err}"),
                    });
                }
            }
            installed += 1;
        } else {
            emit(WorkspaceStatus::ModuleFailed {
                name: dir_name.clone(),
                reason: if !has_lib {
                    format!("No .{} library found", std::env::consts::DLL_EXTENSION)
                } else {
                    "No manifest.toml found".to_string()
                },
            });
        }
    }

    emit(WorkspaceStatus::Done { installed, total });
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{Dependencies, ExtensionManifest};
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Library extension for the running platform (zip installs keep only it).
    const DLL: &str = std::env::consts::DLL_EXTENSION;

    fn manifest(id: &str) -> String {
        format!(
            "[extension]\nid = \"{id}\"\nname = \"Name {id}\"\nversion = \"0.1.0\"\ndescription = \"d\"\n"
        )
    }

    fn write(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    fn collect<T>(rx: mpsc::Receiver<T>) -> Vec<T> {
        rx.iter().collect()
    }

    fn run_ws(ws: &Path, exts: &Path, sel: Option<&[String]>) -> Vec<WorkspaceStatus> {
        let (tx, rx) = mpsc::channel();
        workspace_install_inner(ws.to_path_buf(), exts.to_path_buf(), sel, &tx);
        drop(tx);
        collect(rx)
    }

    fn run_zip(zip: &Path, exts: &Path, sel: Option<&[String]>) -> Vec<WorkspaceStatus> {
        let (tx, rx) = mpsc::channel();
        zip_install_inner(zip.to_path_buf(), exts.to_path_buf(), sel, &tx);
        drop(tx);
        collect(rx)
    }

    fn read_source(dir: &Path) -> ExtensionSource {
        toml::from_str(&std::fs::read_to_string(dir.join("source.toml")).unwrap()).unwrap()
    }

    // ── Small helpers ────────────────────────────────────────────────────

    #[test]
    fn write_source_creates_parseable_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_source(
            tmp.path(),
            &ExtensionSource {
                kind: SourceKind::Workspace,
                path: Some("/ws".into()),
                member: Some("m".into()),
                url: None,
                id: None,
            },
        );
        let s = read_source(tmp.path());
        assert_eq!(s.kind, SourceKind::Workspace);
        assert_eq!(s.member.as_deref(), Some("m"));
    }

    #[test]
    fn parse_workspace_members_variants() {
        assert_eq!(
            parse_workspace_members("[workspace]\nmembers = [\"a\", \"b\", 3]\n").unwrap(),
            vec!["a", "b"]
        );
        assert!(parse_workspace_members("[package]\nname = \"x\"\n").is_err());
        assert!(parse_workspace_members("[workspace]\n").is_err());
        assert!(parse_workspace_members("[workspace]\nmembers = \"a\"\n").is_err());
        assert!(parse_workspace_members("not = [toml").is_err());
    }

    #[test]
    fn discover_modules_keeps_members_with_valid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        write(
            &ws.join("Cargo.toml"),
            b"[workspace]\nmembers = [\"good\", \"bad\", \"none\"]\n",
        );
        write(
            &ws.join("good/manifest.toml"),
            manifest("acme.good").as_bytes(),
        );
        write(&ws.join("bad/manifest.toml"), b"garbage [");
        std::fs::create_dir_all(ws.join("none")).unwrap();
        let mods = discover_modules(ws).unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].member, "good");
        assert_eq!(mods[0].manifest.extension.id, "acme.good");
    }

    #[test]
    fn discover_modules_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_modules(tmp.path()).is_err(), "no Cargo.toml");
        write(&tmp.path().join("Cargo.toml"), b"[package]\nname = \"x\"\n");
        assert!(discover_modules(tmp.path()).is_err(), "not a workspace");
    }

    #[test]
    fn find_lib_in_release_dir_checks_platform_names() {
        let tmp = tempfile::tempdir().unwrap();
        let rel = tmp.path();
        assert!(find_lib_in_release_dir(rel, "my_ext").is_none());
        for name in ["libmy_ext.so", "libmy_ext.dylib", "my_ext.dll"] {
            let t = tempfile::tempdir().unwrap();
            write(&t.path().join(name), b"");
            assert_eq!(
                find_lib_in_release_dir(t.path(), "my_ext"),
                Some(t.path().join(name))
            );
        }
        // Prefers the .so when several exist.
        write(&rel.join("x.dll"), b"");
        write(&rel.join("libx.so"), b"");
        assert_eq!(find_lib_in_release_dir(rel, "x"), Some(rel.join("libx.so")));
    }

    #[test]
    fn find_prebuilt_lib_looks_in_target_release() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_prebuilt_lib(tmp.path(), "e").is_none());
        for name in ["libe.so", "libe.dylib", "e.dll"] {
            let t = tempfile::tempdir().unwrap();
            let p = t.path().join("target").join("release").join(name);
            write(&p, b"");
            assert_eq!(find_prebuilt_lib(t.path(), "e"), Some(p));
        }
    }

    #[test]
    fn find_lib_file_returns_the_platform_library() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_lib_file(&tmp.path().join("missing")).is_none());
        write(&tmp.path().join("build.log"), b"");
        assert!(find_lib_file(tmp.path()).is_none());
        let other = if cfg!(windows) { "foo.so" } else { "foo.dll" };
        write(&tmp.path().join(other), b"");
        assert!(
            find_lib_file(tmp.path()).is_none(),
            "foreign library ignored"
        );
        let native = format!("foo.{}", std::env::consts::DLL_EXTENSION);
        write(&tmp.path().join(&native), b"");
        assert_eq!(find_lib_file(tmp.path()), Some(tmp.path().join(native)));
    }

    #[test]
    fn tempdir_for_clone_uses_repo_name() {
        for url in [
            "https://example.invalid/acme/my-ext.git",
            "https://example.invalid/acme/my-ext/",
            "my-ext",
        ] {
            let p = tempdir_for_clone(url).unwrap();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            assert!(name.starts_with("wu-ext-my-ext-"), "{name}");
            assert!(p.starts_with(std::env::temp_dir()));
            assert!(!p.exists(), "only a path is computed");
        }
        assert_ne!(
            tempdir_for_clone("a").unwrap(),
            tempdir_for_clone("a").unwrap()
        );
    }

    #[test]
    fn install_job_new_is_idle() {
        let j = InstallJob::new("https://example.invalid/x".into());
        assert_eq!(j.status, InstallStatus::Idle);
        assert!(j.log.is_empty());
        assert_eq!(j.repo_url, "https://example.invalid/x");
    }

    #[test]
    fn shell_command_wraps_with_cmd_on_windows() {
        let c = shell_command("npm");
        let args: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        if cfg!(target_os = "windows") {
            assert_eq!(c.get_program(), "cmd");
            assert_eq!(args, vec!["/C", "npm"]);
        } else {
            assert_eq!(c.get_program(), "npm");
            assert!(args.is_empty());
        }
    }

    #[test]
    fn split_pkg_version_variants() {
        assert_eq!(
            split_pkg_version("csharp-ls@0.16.0"),
            ("csharp-ls", Some("0.16.0"))
        );
        assert_eq!(split_pkg_version("pylsp"), ("pylsp", None));
        assert_eq!(split_pkg_version("a@b@c"), ("a", Some("b@c")));
    }

    #[test]
    fn dependency_summary_lists_every_manager() {
        let deps = Dependencies {
            npm: vec!["pyright@1.1".into()],
            pip: vec!["python-lsp-server".into()],
            cargo: vec!["taplo-cli@0.9".into()],
            go: vec!["golang.org/x/tools/gopls@latest".into()],
            dotnet: vec!["csharp-ls@0.16.0".into()],
        };
        assert_eq!(
            dependency_summary(&deps),
            vec![
                "pyright (npm)",
                "python-lsp-server (pip)",
                "taplo-cli (cargo)",
                "golang.org/x/tools/gopls@latest (go)",
                "csharp-ls (dotnet tool)",
            ]
        );
        assert!(dependency_summary(&Dependencies::default()).is_empty());
    }

    #[test]
    fn no_dependencies_means_no_commands() {
        let mut steps = Vec::new();
        let errs = install_deps(&Dependencies::default(), |s| steps.push(s));
        assert!(errs.is_empty());
        assert!(steps.is_empty());
        assert!(uninstall_deps(&Dependencies::default()).is_empty());
    }

    #[test]
    fn run_uninstall_records_failure_with_label() {
        let mut errors = Vec::new();
        run_uninstall(
            &mut errors,
            "fakepm",
            "definitely-not-a-package-manager-xyz",
            &["uninstall", "x"],
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("fakepm"), "{}", errors[0]);
    }

    #[test]
    fn copy_lib_safe_copies_and_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("new.dll");
        let dest = tmp.path().join("out").join("ext.dll");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        write(&src, b"v1");
        copy_lib_safe(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"v1");

        write(&src, b"v2");
        write(&dest.with_extension("old"), b"stale");
        copy_lib_safe(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"v2");
        assert!(!dest.with_extension("old").exists(), ".old cleaned up");

        assert!(copy_lib_safe(&tmp.path().join("missing.dll"), &dest).is_err());
    }

    #[test]
    fn zip_with_unsafe_manifest_ids_is_rejected() {
        for id in ["..", "a/b", "C:\\\\x", "a\\\\..\\\\..", "/abs"] {
            let tmp = tempfile::tempdir().unwrap();
            let exts = tmp.path().join("exts");
            let zip = tmp.path().join("evil.zip");
            let m = manifest(id);
            make_zip(
                &zip,
                &[
                    ("m/manifest.toml", m.as_bytes()),
                    (&format!("m/m.{DLL}"), b"pwned"),
                ],
            );
            // Unsafe manifests are not offered for install at all.
            assert!(discover_zip_modules(&zip).unwrap().is_empty(), "{id}");
            let st = run_zip(&zip, &exts, None);
            assert!(
                matches!(&st[0], WorkspaceStatus::Failed(m) if m.contains("No modules")),
                "{id}: {st:?}"
            );
            assert!(!exts.exists(), "{id}: nothing must be written");
        }
    }

    #[test]
    fn zip_with_nested_backslash_entries_flattens_into_extension_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let exts = tmp.path().join("exts");
        let zip = tmp.path().join("win.zip");
        let m = manifest("acme.m");
        make_zip(
            &zip,
            &[
                ("m\\manifest.toml", m.as_bytes()),
                (&format!("m\\target\\release\\m.{DLL}"), b"1"),
                (&format!("m\\..\\..\\evil.{DLL}"), b"pwned"),
            ],
        );
        let st = run_zip(&zip, &exts, None);
        assert_eq!(
            st.last(),
            Some(&WorkspaceStatus::Done {
                installed: 1,
                total: 1
            })
        );
        assert!(exts.join("acme.m").join(format!("m.{DLL}")).is_file());
        assert!(exts.join("acme.m").join(format!("evil.{DLL}")).is_file());
        assert!(!tmp.path().join(format!("evil.{DLL}")).exists());
        assert!(!exts.join(format!("evil.{DLL}")).exists());
    }

    #[test]
    fn extension_dest_dir_revalidates_id() {
        let mut m: ExtensionManifest = toml::from_str(&manifest("acme.ok")).unwrap();
        let base = Path::new("exts");
        assert_eq!(extension_dest_dir(base, &m).unwrap(), base.join("acme.ok"));
        m.extension.id = "../escape".into();
        assert!(extension_dest_dir(base, &m).is_err());
    }

    // ── Folder install ───────────────────────────────────────────────────

    #[test]
    fn install_from_folder_without_manifest_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let st = collect(install_from_folder(
            tmp.path().to_path_buf(),
            tmp.path().join("exts"),
        ));
        assert_eq!(
            st,
            vec![InstallStatus::Failed(
                "No manifest.toml found in folder".into()
            )]
        );
    }

    #[test]
    fn install_from_folder_with_invalid_manifest_fails() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("manifest.toml"), b"[extension]\nid = 1\n");
        let st = collect(install_from_folder(
            tmp.path().to_path_buf(),
            tmp.path().join("exts"),
        ));
        assert_eq!(st.len(), 1);
        assert!(
            matches!(&st[0], InstallStatus::Failed(m) if m.starts_with("Invalid manifest.toml"))
        );
    }

    #[test]
    fn install_from_folder_rejects_traversal_id() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("my-ext");
        let exts = tmp.path().join("exts");
        write(
            &folder.join("manifest.toml"),
            manifest("../escaped").as_bytes(),
        );
        write(
            &folder.join("target").join("release").join("my_ext.dll"),
            b"binary",
        );
        let st = collect(install_from_folder(folder, exts.clone()));
        assert_eq!(st.len(), 1, "{st:?}");
        assert!(
            matches!(&st[0], InstallStatus::Failed(m) if m.contains("extension id")),
            "{st:?}"
        );
        assert!(!tmp.path().join("escaped").exists());
        assert!(!exts.exists());
    }

    #[test]
    fn install_from_folder_with_prebuilt_library() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("my-ext");
        write(
            &folder.join("manifest.toml"),
            manifest("acme.my-ext").as_bytes(),
        );
        write(
            &folder.join("target").join("release").join("my_ext.dll"),
            b"binary",
        );
        let exts = tmp.path().join("exts");
        let st = collect(install_from_folder(folder.clone(), exts.clone()));
        assert_eq!(st, vec![InstallStatus::Installing, InstallStatus::Done]);
        let dest = exts.join("acme.my-ext");
        assert_eq!(std::fs::read(dest.join("my_ext.dll")).unwrap(), b"binary");
        let m: ExtensionManifest =
            toml::from_str(&std::fs::read_to_string(dest.join("manifest.toml")).unwrap()).unwrap();
        assert_eq!(m.extension.id, "acme.my-ext");
        let src = read_source(&dest);
        assert_eq!(src.kind, SourceKind::Folder);
        assert_eq!(src.path.as_deref(), Some(folder.to_string_lossy().as_ref()));

        // Re-installing over an existing install replaces the library.
        write(
            &folder.join("target").join("release").join("my_ext.dll"),
            b"binary-v2",
        );
        let st = collect(install_from_folder(folder, exts));
        assert_eq!(st.last(), Some(&InstallStatus::Done));
        assert_eq!(
            std::fs::read(dest.join("my_ext.dll")).unwrap(),
            b"binary-v2"
        );
    }

    #[test]
    fn install_from_folder_reports_mkdir_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("e");
        write(&folder.join("manifest.toml"), manifest("acme.e").as_bytes());
        write(&folder.join("target/release/e.dll"), b"x");
        // extensions_dir is a regular file → create_dir_all fails.
        let exts = tmp.path().join("exts-file");
        write(&exts, b"");
        let st = collect(install_from_folder(folder, exts));
        assert_eq!(st[0], InstallStatus::Installing);
        assert!(matches!(&st[1], InstallStatus::Failed(m) if m.starts_with("Cannot create dir")));
    }

    // ── Workspace / git install (pre-build error paths only) ─────────────

    #[test]
    fn workspace_install_without_cargo_toml_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let st = run_ws(tmp.path(), &tmp.path().join("exts"), None);
        assert_eq!(st.len(), 1);
        assert!(
            matches!(&st[0], WorkspaceStatus::Failed(m) if m.starts_with("Cannot read Cargo.toml"))
        );
        // Thread wrapper behaves the same.
        let st = collect(install_from_workspace(
            tmp.path().to_path_buf(),
            tmp.path().join("exts"),
            None,
        ));
        assert!(matches!(&st[0], WorkspaceStatus::Failed(_)));
    }

    #[test]
    fn workspace_install_with_non_workspace_cargo_toml_fails() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("Cargo.toml"), b"[package]\nname = \"x\"\n");
        let st = run_ws(tmp.path(), &tmp.path().join("exts"), None);
        assert!(matches!(&st[0], WorkspaceStatus::Failed(m) if m.starts_with("Invalid workspace")));
    }

    #[test]
    fn workspace_install_without_matching_modules_fails_before_building() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        write(
            &ws.join("Cargo.toml"),
            b"[workspace]\nmembers = [\"a\", \"b\"]\n",
        );
        write(&ws.join("a/manifest.toml"), manifest("acme.a").as_bytes());
        std::fs::create_dir_all(ws.join("b")).unwrap();
        let only_b = vec!["b".to_string()];
        let st = run_ws(ws, &ws.join("exts"), Some(&only_b));
        assert_eq!(
            st,
            vec![WorkspaceStatus::Failed(
                "No modules with manifest.toml found in this workspace.".into()
            )]
        );
    }

    #[test]
    fn single_git_install_error_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        single_git_install_inner(tmp.path(), &tmp.path().join("exts"), "u", &tx);
        write(&tmp.path().join("manifest.toml"), b"nope [");
        single_git_install_inner(tmp.path(), &tmp.path().join("exts"), "u", &tx);
        drop(tx);
        let st = collect(rx);
        assert_eq!(st.len(), 4);
        assert_eq!(st[0], WorkspaceStatus::Building);
        assert!(matches!(&st[1], WorkspaceStatus::Failed(m) if m.starts_with("No manifest.toml")));
        assert_eq!(st[2], WorkspaceStatus::Building);
        assert!(matches!(&st[3], WorkspaceStatus::Failed(m) if m.starts_with("Invalid manifest")));
    }

    // ── ZIP ──────────────────────────────────────────────────────────────

    #[test]
    fn discover_zip_modules_finds_top_level_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("bundle.zip");
        let a = manifest("acme.a");
        let deep = manifest("acme.deep");
        let root = manifest("acme.root");
        make_zip(
            &zip,
            &[
                ("mod-a/", b""),
                ("mod-a/manifest.toml", a.as_bytes()),
                ("mod-b/manifest.toml", b"broken ["),
                ("deep/nested/manifest.toml", deep.as_bytes()),
                ("manifest.toml", root.as_bytes()),
            ],
        );
        let mods = discover_zip_modules(&zip).unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].dir_name, "mod-a");
        assert_eq!(mods[0].manifest.extension.id, "acme.a");
    }

    #[test]
    fn discover_zip_modules_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_zip_modules(&tmp.path().join("missing.zip")).is_err());
        let bogus = tmp.path().join("bogus.zip");
        write(&bogus, b"not a zip");
        assert!(discover_zip_modules(&bogus).is_err());
    }

    #[test]
    fn install_from_zip_installs_complete_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("bundle.zip");
        let (ma, mb, mc) = (manifest("acme.a"), manifest("acme.b"), manifest("acme.c"));
        make_zip(
            &zip,
            &[
                ("mod-a/", b""),
                ("mod-a/manifest.toml", ma.as_bytes()),
                ("mod-a/mod_a.dll", b"dll-bytes"),
                ("mod-a/libmod_a.so", b"so-bytes"),
                ("mod-a/libmod_a.dylib", b"dylib-bytes"),
                ("mod-a/README.md", b"ignored"),
                ("mod-a/src/", b""),
                ("mod-b/manifest.toml", mb.as_bytes()),
                ("mod-c/manifest.toml", mc.as_bytes()),
            ],
        );
        let exts = tmp.path().join("exts");
        let st = collect(install_from_zip(zip.clone(), exts.clone(), None));
        let installing = |n: &str, done| WorkspaceStatus::Installing {
            current: format!("Name {n}"),
            done,
            total: 3,
        };
        let no_lib = |n: &str| WorkspaceStatus::ModuleFailed {
            name: n.into(),
            reason: format!("No .{} library found", std::env::consts::DLL_EXTENSION),
        };
        assert_eq!(
            st,
            vec![
                installing("acme.a", 0),
                installing("acme.b", 1),
                no_lib("mod-b"),
                installing("acme.c", 2),
                no_lib("mod-c"),
                WorkspaceStatus::Done {
                    installed: 1,
                    total: 3
                },
            ]
        );
        let a = exts.join("acme.a");
        // Only the library for the running platform is extracted.
        let libs = [
            ("mod_a.dll", "dll", b"dll-bytes".as_slice()),
            ("libmod_a.so", "so", b"so-bytes".as_slice()),
            ("libmod_a.dylib", "dylib", b"dylib-bytes".as_slice()),
        ];
        for (name, ext, bytes) in libs {
            if ext == std::env::consts::DLL_EXTENSION {
                assert_eq!(std::fs::read(a.join(name)).unwrap(), bytes);
            } else {
                assert!(!a.join(name).exists(), "{name} is for another platform");
            }
        }
        assert!(a.join("manifest.toml").is_file());
        assert!(
            !a.join("README.md").exists(),
            "only libs + manifest extracted"
        );
        assert!(!a.join("mod_a.dll.tmp").exists(), "temp file removed");
        let src = read_source(&a);
        assert_eq!(src.kind, SourceKind::Zip);
        assert_eq!(src.member.as_deref(), Some("mod-a"));
        assert!(!exts.join("acme.b").join("source.toml").exists());
    }

    #[test]
    fn install_from_zip_respects_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("bundle.zip");
        let (ma, mb) = (manifest("acme.a"), manifest("acme.b"));
        make_zip(
            &zip,
            &[
                ("a/manifest.toml", ma.as_bytes()),
                (&format!("a/a.{DLL}"), b"1"),
                ("b/manifest.toml", mb.as_bytes()),
                (&format!("b/b.{DLL}"), b"2"),
            ],
        );
        let exts = tmp.path().join("exts");
        let sel = vec!["b".to_string()];
        let st = run_zip(&zip, &exts, Some(&sel));
        assert_eq!(
            st.last(),
            Some(&WorkspaceStatus::Done {
                installed: 1,
                total: 1
            })
        );
        assert!(exts.join("acme.b").join(format!("b.{DLL}")).is_file());
        assert!(!exts.join("acme.a").exists());

        let none = vec!["zzz".to_string()];
        assert_eq!(
            run_zip(&zip, &exts, Some(&none)),
            vec![WorkspaceStatus::Failed(
                "No modules with manifest.toml found in this ZIP.".into()
            )]
        );
    }

    #[test]
    fn install_from_zip_invalid_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let st = run_zip(&tmp.path().join("missing.zip"), tmp.path(), None);
        assert!(matches!(&st[0], WorkspaceStatus::Failed(m) if m.starts_with("Cannot open ZIP")));
        let bogus = tmp.path().join("bogus.zip");
        write(&bogus, b"PK not really");
        let st = run_zip(&bogus, tmp.path(), None);
        assert!(matches!(&st[0], WorkspaceStatus::Failed(m) if m.starts_with("Invalid ZIP")));
        let empty = tmp.path().join("empty.zip");
        make_zip(&empty, &[("readme.txt", b"hi")]);
        let st = run_zip(&empty, tmp.path(), None);
        assert!(matches!(&st[0], WorkspaceStatus::Failed(m) if m.contains("No modules")));
    }

    #[test]
    fn zip_entry_path_traversal_stays_inside_extension_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let exts = root.join("exts");
        let zip = tmp.path().join("evil.zip");
        let m = manifest("acme.m");
        make_zip(
            &zip,
            &[
                ("m/manifest.toml", m.as_bytes()),
                (&format!("m/../../evil.{DLL}"), b"pwned"),
                (&format!("m/../../../evil2.{DLL}"), b"pwned"),
                ("m/sub/../../manifest.toml", m.as_bytes()),
            ],
        );
        let st = run_zip(&zip, &exts, None);
        assert_eq!(
            st.last(),
            Some(&WorkspaceStatus::Done {
                installed: 1,
                total: 1
            })
        );
        // Entries are flattened to their file name inside the extension dir.
        assert!(exts.join("acme.m").join(format!("evil.{DLL}")).is_file());
        assert!(!root.join(format!("evil.{DLL}")).exists());
        assert!(!tmp.path().join(format!("evil.{DLL}")).exists());
        assert!(!tmp.path().join(format!("evil2.{DLL}")).exists());
        assert!(!exts.join(format!("evil.{DLL}")).exists());
    }

    #[test]
    fn manifest_id_cannot_escape_extensions_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let exts = tmp.path().join("exts");
        let zip = tmp.path().join("evil.zip");
        let m = manifest("../escaped");
        make_zip(
            &zip,
            &[
                ("m/manifest.toml", m.as_bytes()),
                (&format!("m/m.{DLL}"), b"pwned"),
            ],
        );
        let _ = run_zip(&zip, &exts, None);
        assert!(
            !tmp.path().join("escaped").exists(),
            "install wrote outside the extensions directory"
        );
    }

    #[test]
    fn zip_with_backslash_separators_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("win.zip");
        let m = manifest("acme.m");
        make_zip(
            &zip,
            &[
                ("m\\manifest.toml", m.as_bytes()),
                (&format!("m\\m.{DLL}"), b"1"),
            ],
        );
        assert_eq!(discover_zip_modules(&zip).unwrap().len(), 1);
        let st = run_zip(&zip, &tmp.path().join("exts"), None);
        assert_eq!(
            st.last(),
            Some(&WorkspaceStatus::Done {
                installed: 1,
                total: 1
            })
        );
    }
}
