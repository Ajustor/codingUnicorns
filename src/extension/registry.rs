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
                SourceKind::Git | SourceKind::Zip => continue, // requires re-download — skip
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn manifest(id: &str, version: &str) -> String {
        format!(
            "[extension]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\ndescription = \"d\"\n"
        )
    }

    /// Registry rooted in a temp dir — never touches the real config dir.
    fn registry(root: &Path) -> ExtensionRegistry {
        ExtensionRegistry {
            installed: Vec::new(),
            extensions_dir: root.to_path_buf(),
        }
    }

    fn install(root: &Path, dir: &str, manifest_toml: &str) -> PathBuf {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.toml"), manifest_toml).unwrap();
        d
    }

    #[test]
    fn extensions_dir_is_under_coding_unicorns() {
        let p = ExtensionRegistry::extensions_dir();
        assert!(p.ends_with(Path::new("coding-unicorns").join("extensions")));
        let r = ExtensionRegistry::default();
        assert_eq!(r.extensions_dir, p);
        assert!(r.installed.is_empty(), "new() does not scan the disk");
    }

    #[test]
    fn load_installed_missing_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = registry(&tmp.path().join("does-not-exist"));
        r.load_installed();
        assert!(r.installed.is_empty());
    }

    #[test]
    fn load_installed_reads_valid_extensions_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let good = install(root, "acme.good", &manifest("acme.good", "1.0.0"));
        std::fs::write(good.join("acme_good.dll"), b"").unwrap();
        std::fs::write(
            good.join("source.toml"),
            "kind = \"folder\"\npath = \"/src/good\"\n",
        )
        .unwrap();
        install(root, "acme.nolib", &manifest("acme.nolib", "0.1.0"));
        install(root, "broken", "this is not toml [");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("stray-file.txt"), b"x").unwrap();
        let badsrc = install(root, "acme.badsrc", &manifest("acme.badsrc", "0.1.0"));
        std::fs::write(badsrc.join("source.toml"), "kind = 7").unwrap();

        let mut r = registry(root);
        r.installed.push(r_dummy());
        r.load_installed();
        let mut ids: Vec<&str> = r
            .installed
            .iter()
            .map(|e| e.manifest.extension.id.as_str())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["acme.badsrc", "acme.good", "acme.nolib"]);

        let g = r
            .installed
            .iter()
            .find(|e| e.manifest.extension.id == "acme.good")
            .unwrap();
        assert!(g.enabled);
        assert_eq!(g.path, good);
        assert_eq!(
            g.lib_path.as_deref(),
            Some(good.join("acme_good.dll").as_path())
        );
        let src = g.source.as_ref().unwrap();
        assert_eq!(src.kind, super::super::manifest::SourceKind::Folder);
        assert_eq!(src.path.as_deref(), Some("/src/good"));
        assert!(g.update_available.is_none());

        let n = r
            .installed
            .iter()
            .find(|e| e.manifest.extension.id == "acme.nolib")
            .unwrap();
        assert!(n.lib_path.is_none());
        assert!(n.source.is_none());
        let b = r
            .installed
            .iter()
            .find(|e| e.manifest.extension.id == "acme.badsrc")
            .unwrap();
        assert!(b.source.is_none(), "unparseable source.toml is ignored");

        assert!(r.is_installed("acme.good"));
        assert!(
            !r.is_installed("dummy"),
            "load_installed clears old entries"
        );
    }

    fn r_dummy() -> InstalledExtension {
        InstalledExtension {
            manifest: toml::from_str(&manifest("dummy", "0.0.1")).unwrap(),
            path: PathBuf::from("nowhere"),
            lib_path: None,
            enabled: false,
            source: None,
            update_available: None,
        }
    }

    #[test]
    fn find_lib_recognises_all_platform_extensions() {
        for ext in ["so", "dll", "dylib"] {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("readme.md"), b"").unwrap();
            std::fs::write(tmp.path().join(format!("libx.{ext}")), b"").unwrap();
            assert_eq!(
                ExtensionRegistry::find_lib(tmp.path()),
                Some(tmp.path().join(format!("libx.{ext}")))
            );
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("noext"), b"").unwrap();
        assert!(ExtensionRegistry::find_lib(tmp.path()).is_none());
        assert!(ExtensionRegistry::find_lib(&tmp.path().join("missing")).is_none());
    }

    #[test]
    fn uninstall_removes_directory_and_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let d = install(tmp.path(), "acme.x", &manifest("acme.x", "1.0.0"));
        let mut r = registry(tmp.path());
        r.load_installed();
        assert!(r.is_installed("acme.x"));
        r.uninstall("acme.x").unwrap();
        assert!(!r.is_installed("acme.x"));
        assert!(!d.exists());
        // Unknown id is a no-op.
        r.uninstall("acme.unknown").unwrap();
    }

    #[test]
    fn uninstall_reports_fs_errors() {
        let mut r = registry(Path::new("unused"));
        let mut e = r_dummy();
        e.manifest.extension.id = "ghost".into();
        e.path = std::env::temp_dir().join(format!("cu-missing-{}", uuid::Uuid::new_v4()));
        r.installed.push(e);
        assert!(r.uninstall("ghost").is_err());
        assert!(
            !r.is_installed("ghost"),
            "entry removed even if delete fails"
        );
    }

    #[test]
    fn version_gt_compares_numerically() {
        assert!(version_gt("1.0.1", "1.0.0"));
        assert!(version_gt("1.10.0", "1.9.9"));
        assert!(version_gt("2.0.0", "1.99.99"));
        assert!(version_gt(" 0.2 ", "0.1.9"));
        assert!(!version_gt("1.0.0", "1.0.0"));
        assert!(!version_gt("1.0.0", "1.0.1"));
        assert!(!version_gt("garbage", "0.0.0"));
        assert!(version_gt("1", "0.9"));
    }

    fn with_source(
        r: &mut ExtensionRegistry,
        id: &str,
        version: &str,
        source: Option<ExtensionSource>,
    ) {
        r.installed.push(InstalledExtension {
            manifest: toml::from_str(&manifest(id, version)).unwrap(),
            path: PathBuf::new(),
            lib_path: None,
            enabled: true,
            source,
            update_available: Some("stale".into()),
        });
    }

    fn src(
        kind: super::super::manifest::SourceKind,
        path: Option<&Path>,
        member: Option<&str>,
    ) -> Option<ExtensionSource> {
        Some(ExtensionSource {
            kind,
            path: path.map(|p| p.to_string_lossy().to_string()),
            member: member.map(|m| m.to_string()),
            url: None,
        })
    }

    #[test]
    fn check_updates_compares_against_local_sources() {
        use super::super::manifest::SourceKind;
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        install(&ws, "rust-lang", &manifest("acme.rust", "1.1.0"));
        let folder = install(tmp.path(), "folder", &manifest("acme.folder", "0.5.0"));
        let broken = install(tmp.path(), "broken", "not toml [");

        let mut r = registry(tmp.path());
        with_source(
            &mut r,
            "acme.rust",
            "1.0.0",
            src(SourceKind::Workspace, Some(&ws), Some("rust-lang")),
        );
        with_source(
            &mut r,
            "acme.folder",
            "0.5.0",
            src(SourceKind::Folder, Some(&folder), None),
        );
        with_source(
            &mut r,
            "acme.git",
            "0.1.0",
            src(SourceKind::Git, None, None),
        );
        with_source(
            &mut r,
            "acme.zip",
            "0.1.0",
            src(SourceKind::Zip, Some(&folder), None),
        );
        with_source(&mut r, "acme.nosrc", "0.1.0", None);
        with_source(
            &mut r,
            "acme.ws-nopath",
            "0.1.0",
            src(SourceKind::Workspace, None, Some("m")),
        );
        with_source(
            &mut r,
            "acme.ws-nomember",
            "0.1.0",
            src(SourceKind::Workspace, Some(&ws), None),
        );
        with_source(
            &mut r,
            "acme.folder-nopath",
            "0.1.0",
            src(SourceKind::Folder, None, None),
        );
        with_source(
            &mut r,
            "acme.missing",
            "0.1.0",
            src(SourceKind::Folder, Some(&tmp.path().join("nope")), None),
        );
        with_source(
            &mut r,
            "acme.broken",
            "0.1.0",
            src(SourceKind::Folder, Some(&broken), None),
        );

        r.check_updates();
        let upd: Vec<(&str, Option<&str>)> = r
            .installed
            .iter()
            .map(|e| {
                (
                    e.manifest.extension.id.as_str(),
                    e.update_available.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            upd,
            vec![
                ("acme.rust", Some("1.1.0")),
                ("acme.folder", None),
                ("acme.git", None),
                ("acme.zip", None),
                ("acme.nosrc", None),
                ("acme.ws-nopath", None),
                ("acme.ws-nomember", None),
                ("acme.folder-nopath", None),
                ("acme.missing", None),
                ("acme.broken", None),
            ]
        );
    }
}
