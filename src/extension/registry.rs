use super::manifest::{ExtensionManifest, ExtensionSource};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct InstalledExtension {
    pub manifest: ExtensionManifest,
    pub path: PathBuf,
    pub lib_path: Option<PathBuf>,
    pub enabled: bool,
    /// Where this extension was installed from (written to `source.toml`).
    pub source: Option<ExtensionSource>,
    /// Set to the newer version string when an update is detected.
    pub update_available: Option<String>,
}

pub struct ExtensionRegistry {
    pub installed: Vec<InstalledExtension>,
    pub extensions_dir: PathBuf,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        let extensions_dir = Self::extensions_dir();
        Self {
            installed: Vec::new(),
            extensions_dir,
        }
    }

    pub fn extensions_dir() -> PathBuf {
        dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("coding-unicorns")
            .join("extensions")
    }

    pub fn load_installed(&mut self) {
        self.installed.clear();
        let dir = self.extensions_dir.clone();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.toml");
            let Ok(content) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = toml::from_str::<ExtensionManifest>(&content) else {
                continue;
            };
            let lib_path = Self::find_lib(&path);
            let source = std::fs::read_to_string(path.join("source.toml"))
                .ok()
                .and_then(|s| toml::from_str::<ExtensionSource>(&s).ok());
            self.installed.push(InstalledExtension {
                manifest,
                path,
                lib_path,
                enabled: true,
                source,
                update_available: None,
            });
        }
    }

    fn find_lib(dir: &std::path::Path) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let p = entry.path();
            if let Some(ext) = p.extension() {
                if ext == "so" || ext == "dll" || ext == "dylib" {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Check all installed extensions for available updates (compares version strings).
    /// Updates `update_available` in-place. Non-blocking — reads only local files.
    pub fn check_updates(&mut self) {
        use super::manifest::SourceKind;
        for ext in &mut self.installed {
            ext.update_available = None;
            let Some(source) = &ext.source else { continue };
            let source_manifest_path = match &source.kind {
                SourceKind::Workspace => {
                    let Some(path) = &source.path else { continue };
                    let Some(member) = &source.member else {
                        continue;
                    };
                    PathBuf::from(path).join(member).join("manifest.toml")
                }
                SourceKind::Folder => {
                    let Some(path) = &source.path else { continue };
                    PathBuf::from(path).join("manifest.toml")
                }
                SourceKind::Git => continue, // git requires network — skip
            };
            let Ok(content) = std::fs::read_to_string(&source_manifest_path) else {
                continue;
            };
            let Ok(source_manifest) = toml::from_str::<ExtensionManifest>(&content) else {
                continue;
            };
            let src_ver = &source_manifest.extension.version;
            let cur_ver = &ext.manifest.extension.version;
            if version_gt(src_ver, cur_ver) {
                ext.update_available = Some(src_ver.clone());
            }
        }
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.installed.iter().any(|e| e.manifest.extension.id == id)
    }

    pub fn uninstall(&mut self, id: &str) -> anyhow::Result<()> {
        if let Some(pos) = self
            .installed
            .iter()
            .position(|e| e.manifest.extension.id == id)
        {
            let ext = self.installed.remove(pos);
            std::fs::remove_dir_all(&ext.path)?;
        }
        Ok(())
    }
}

/// Returns true if `a` is a strictly greater semver than `b`.
fn version_gt(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> (u32, u32, u32) {
        let mut parts = v.trim().splitn(3, '.');
        let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    }
    parse(a) > parse(b)
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
