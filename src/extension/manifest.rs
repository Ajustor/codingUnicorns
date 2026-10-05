use serde::{Deserialize, Serialize};

// ── Source tracking ───────────────────────────────────────────────────────────

/// Serialised to `source.toml` next to `manifest.toml` in the installed dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionSource {
    pub kind: SourceKind,
    /// Workspace root or local folder path.
    #[serde(default)]
    pub path: Option<String>,
    /// Cargo workspace member name (only for `Workspace` kind).
    #[serde(default)]
    pub member: Option<String>,
    /// Git remote URL (only for `Git` kind).
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Workspace,
    Folder,
    Git,
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub extension: ExtensionInfo,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub dependencies: Dependencies,
}

/// External tools/packages that must be installed for this module to work.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    /// npm packages to install globally (`npm install -g <pkg>`).
    #[serde(default)]
    pub npm: Vec<String>,
    /// Python packages to install (`pip3 install <pkg>`).
    #[serde(default)]
    pub pip: Vec<String>,
    /// Cargo crates to install (`cargo install <pkg>`).
    #[serde(default)]
    pub cargo: Vec<String>,
    /// Go tools to install (`go install <pkg>`).
    #[serde(default)]
    pub go: Vec<String>,
    /// .NET global tools to install (`dotnet tool update --global <pkg>`).
    /// Pin a version with `name@version`, e.g. `csharp-ls@0.16.0`.
    #[serde(default)]
    pub dotnet: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repository: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub themes: Vec<String>,
    /// LSP server binary name (e.g. "rust-analyzer", "pylsp").
    #[serde(default)]
    pub lsp_server: Option<String>,
    /// Arguments to pass to the LSP server (e.g. ["--stdio"]).
    #[serde(default)]
    pub lsp_args: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[extension]
id = "acme.python"
name = "Python"
version = "1.2.3"
description = "Python support"
author = "Acme"
repository = "https://example.invalid/python"

[capabilities]
languages = ["py", "pyw"]
commands = ["python.run"]
themes = ["dark"]
lsp_server = "pylsp"
lsp_args = ["--stdio"]

[dependencies]
pip = ["python-lsp-server"]
npm = ["pyright"]
cargo = ["ruff"]
go = ["golang.org/x/tools/gopls@latest"]
dotnet = ["csharp-ls@0.16.0"]
"#;

    #[test]
    fn parses_full_manifest() {
        let m: ExtensionManifest = toml::from_str(FULL).unwrap();
        assert_eq!(m.extension.id, "acme.python");
        assert_eq!(m.extension.version, "1.2.3");
        assert_eq!(m.extension.author, "Acme");
        assert_eq!(m.capabilities.languages, vec!["py", "pyw"]);
        assert_eq!(m.capabilities.commands, vec!["python.run"]);
        assert_eq!(m.capabilities.themes, vec!["dark"]);
        assert_eq!(m.capabilities.lsp_server.as_deref(), Some("pylsp"));
        assert_eq!(m.capabilities.lsp_args, vec!["--stdio"]);
        assert_eq!(m.dependencies.pip, vec!["python-lsp-server"]);
        assert_eq!(m.dependencies.npm, vec!["pyright"]);
        assert_eq!(m.dependencies.cargo, vec!["ruff"]);
        assert_eq!(m.dependencies.go.len(), 1);
        assert_eq!(m.dependencies.dotnet, vec!["csharp-ls@0.16.0"]);
    }

    #[test]
    fn optional_sections_default() {
        let m: ExtensionManifest = toml::from_str(
            r#"[extension]
id = "a.b"
name = "B"
version = "0.1.0"
description = "d"
"#,
        )
        .unwrap();
        assert_eq!(m.extension.author, "");
        assert_eq!(m.extension.repository, "");
        assert!(m.capabilities.languages.is_empty());
        assert!(m.capabilities.lsp_server.is_none());
        assert!(m.capabilities.lsp_args.is_empty());
        assert!(m.dependencies.npm.is_empty() && m.dependencies.dotnet.is_empty());
    }

    #[test]
    fn missing_required_fields_are_rejected() {
        assert!(toml::from_str::<ExtensionManifest>("").is_err());
        assert!(toml::from_str::<ExtensionManifest>(
            "[extension]\nid = \"a\"\nname = \"n\"\nversion = \"1\"\n"
        )
        .is_err());
        assert!(toml::from_str::<ExtensionManifest>(
            "[extension]\nid = 5\nname = \"n\"\nversion = \"1\"\ndescription = \"\"\n"
        )
        .is_err());
    }

    #[test]
    fn manifest_roundtrips_through_toml() {
        let m: ExtensionManifest = toml::from_str(FULL).unwrap();
        let s = toml::to_string(&m).unwrap();
        let back: ExtensionManifest = toml::from_str(&s).unwrap();
        assert_eq!(back.extension.id, m.extension.id);
        assert_eq!(back.capabilities.lsp_args, m.capabilities.lsp_args);
        assert_eq!(back.dependencies.go, m.dependencies.go);
    }

    #[test]
    fn source_kinds_serialize_lowercase() {
        for (kind, s) in [
            (SourceKind::Workspace, "workspace"),
            (SourceKind::Folder, "folder"),
            (SourceKind::Git, "git"),
            (SourceKind::Zip, "zip"),
        ] {
            let src = ExtensionSource {
                kind: kind.clone(),
                path: None,
                member: None,
                url: None,
            };
            let out = toml::to_string(&src).unwrap();
            assert!(out.contains(&format!("kind = \"{s}\"")), "{out}");
            let back: ExtensionSource = toml::from_str(&out).unwrap();
            assert_eq!(back.kind, kind);
        }
    }

    #[test]
    fn source_optional_fields_default_to_none() {
        let s: ExtensionSource = toml::from_str("kind = \"git\"\nurl = \"u\"").unwrap();
        assert_eq!(s.kind, SourceKind::Git);
        assert_eq!(s.url.as_deref(), Some("u"));
        assert!(s.path.is_none() && s.member.is_none());
        assert!(toml::from_str::<ExtensionSource>("kind = \"svn\"").is_err());
    }
}
