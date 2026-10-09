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
    /// Git remote URL (`Git` kind) or registry index URL (`Registry` kind).
    #[serde(default)]
    pub url: Option<String>,
    /// Module id in the remote registry index (only for `Registry` kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Workspace,
    Folder,
    Git,
    Zip,
    /// Downloaded from the remote module registry (`url` = index, `id` = module).
    Registry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub extension: ExtensionInfo,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub dependencies: Dependencies,
    /// Debug adapter (DAP) of the language, if the module ships one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debugger: Option<DebuggerSpec>,
}

/// `[debugger]` section: how to start the language's debug adapter (DAP
/// over stdio, or over a local TCP port).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebuggerSpec {
    /// VS Code debug `type`s handled (e.g. `["coreclr"]`), matched against
    /// launch configurations.
    #[serde(default)]
    pub types: Vec<String>,
    /// Executable, or candidates tried in order, looked up on `PATH`.
    pub command: OneOrMany,
    /// Arguments. `${debuggerDir}` is the folder the download is unpacked
    /// in, `${port}` the TCP port to listen on (`transport = "tcp"`).
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub transport: DebuggerTransport,
    /// Launch configuration `type` rewrites (e.g. `node = "pwa-node"`) for
    /// adapters that only know some of the names VS Code accepts.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub type_map: std::collections::BTreeMap<String, String>,
    /// Launch arguments used without a launch configuration (e.g.
    /// `{ program = "${file}" }`). Without it a launch configuration is
    /// required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_launch: Option<toml::Table>,
    /// Shown when the adapter is missing and cannot be downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
    /// Archive unpacked in the module's folder (`${debuggerDir}`): on first
    /// use when `args` refer to `${debuggerDir}`, else when the command is
    /// not on `PATH` (the downloaded binary is then the command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<DebuggerDownload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebuggerDownload {
    /// File inside the archive (`/`-separated) whose presence means the
    /// download is installed; an executable is given without `.exe`
    /// (e.g. `netcoredbg/netcoredbg`).
    pub binary: String,
    /// `.zip` or `.tar.gz` URL per platform key (`windows-x86_64`,
    /// `linux-x86_64`, `macos-aarch64`…).
    pub urls: std::collections::BTreeMap<String, String>,
}

/// How the IDE talks to a debug adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DebuggerTransport {
    /// DAP on the adapter's stdin/stdout.
    #[default]
    Stdio,
    /// The adapter listens on `127.0.0.1:${port}`; the IDE connects to it.
    /// Child sessions (`startDebugging`) open more connections.
    Tcp,
}

/// A string or a list of strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn as_slice(&self) -> &[String] {
        match self {
            OneOrMany::One(s) => std::slice::from_ref(s),
            OneOrMany::Many(v) => v,
        }
    }
}

impl ExtensionManifest {
    /// Parse a `manifest.toml` and validate it. Every install path must use
    /// this (not raw `toml::from_str`) because `extension.id` becomes a
    /// directory name under the extensions dir.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(s)?;
        validate_extension_id(&manifest.extension.id)?;
        Ok(manifest)
    }
}

/// Reject extension ids that are not a single, safe path component.
/// Allowed: non-empty, only `[A-Za-z0-9._-]`, and not `.` / `..`.
/// This rules out separators (`/`, `\`), drive prefixes (`C:`), and
/// parent-directory traversal.
pub fn validate_extension_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() {
        anyhow::bail!("extension id must not be empty");
    }
    if id == "." || id == ".." {
        anyhow::bail!("extension id `{id}` is not allowed");
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        anyhow::bail!(
            "extension id `{id}` contains invalid character `{c}` (allowed: A-Z a-z 0-9 . _ -)"
        );
    }
    Ok(())
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
    /// `initializationOptions` sent to the LSP server (e.g.
    /// `{ provideFormatter = true }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_init_options: Option<toml::Table>,
    /// LSP `languageId` per file extension when it differs from the
    /// extension (e.g. `{ cs = "csharp" }`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub language_ids: std::collections::BTreeMap<String, String>,
    /// Languages of files recognised by their name rather than their
    /// extension, matched case-insensitively, `*` as a wildcard (e.g.
    /// `{ "docker-compose.yml" = "compose", "Containerfile" = "dockerfile" }`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub file_names: std::collections::BTreeMap<String, String>,
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
    fn validate_extension_id_accepts_safe_ids() {
        for id in ["acme.python", "a", "my-ext_2", "A.B.C", "...a", "a.."] {
            assert!(validate_extension_id(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn validate_extension_id_rejects_unsafe_ids() {
        for id in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            r"a\b",
            "/abs",
            r"\abs",
            r"C:\x",
            "C:x",
            r"a\..\..",
            "a/../..",
            "sp ace",
            "nul\0",
            "\u{e9}t\u{e9}",
        ] {
            assert!(validate_extension_id(id).is_err(), "{id:?}");
        }
    }

    #[test]
    fn parse_rejects_traversal_id_and_accepts_valid() {
        let ok = ExtensionManifest::parse(FULL).unwrap();
        assert_eq!(ok.extension.id, "acme.python");
        let bad = FULL.replace("acme.python", "../evil");
        let err = ExtensionManifest::parse(&bad).unwrap_err().to_string();
        assert!(err.contains("extension id"), "{err}");
        assert!(ExtensionManifest::parse("").is_err());
    }

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
        assert!(m.capabilities.language_ids.is_empty());
        let m = ExtensionManifest::parse(&FULL.replace(
            "lsp_args = [\"--stdio\"]",
            "lsp_args = []\nlanguage_ids = { py = \"python\" }",
        ))
        .unwrap();
        assert_eq!(m.capabilities.language_ids["py"], "python");
        assert_eq!(m.dependencies.pip, vec!["python-lsp-server"]);
        assert_eq!(m.dependencies.npm, vec!["pyright"]);
        assert_eq!(m.dependencies.cargo, vec!["ruff"]);
        assert_eq!(m.dependencies.go.len(), 1);
        assert_eq!(m.dependencies.dotnet, vec!["csharp-ls@0.16.0"]);
    }

    #[test]
    fn parses_debugger_section() {
        let m = ExtensionManifest::parse(&format!(
            r#"{FULL}
[debugger]
types = ["coreclr"]
command = "netcoredbg"
args = ["--interpreter=vscode"]
install_hint = "get it"

[debugger.default_launch]
program = "${{file}}"

[debugger.download]
binary = "netcoredbg/netcoredbg"
[debugger.download.urls]
windows-x86_64 = "https://example.invalid/w.zip"
"#
        ))
        .unwrap();
        let d = m.debugger.unwrap();
        assert_eq!(d.types, ["coreclr"]);
        assert_eq!(d.command.as_slice(), ["netcoredbg"]);
        assert_eq!(d.args, ["--interpreter=vscode"]);
        assert_eq!(
            d.default_launch.unwrap()["program"].as_str(),
            Some("${file}")
        );
        let dl = d.download.unwrap();
        assert_eq!(dl.binary, "netcoredbg/netcoredbg");
        assert_eq!(dl.urls["windows-x86_64"], "https://example.invalid/w.zip");

        let m = ExtensionManifest::parse(&format!(
            r#"{FULL}
[debugger]
command = ["python", "python3"]
"#
        ))
        .unwrap();
        let d = m.debugger.unwrap();
        assert_eq!(d.command.as_slice(), ["python", "python3"]);
        assert!(d.types.is_empty() && d.download.is_none());
        assert_eq!(d.transport, DebuggerTransport::Stdio);
        assert!(d.type_map.is_empty());

        let m = ExtensionManifest::parse(&format!(
            r#"{FULL}
[debugger]
command = "node"
transport = "tcp"
type_map = {{ node = "pwa-node" }}
"#
        ))
        .unwrap();
        let d = m.debugger.unwrap();
        assert_eq!(d.transport, DebuggerTransport::Tcp);
        assert_eq!(d.type_map["node"], "pwa-node");
        assert!(ExtensionManifest::parse(FULL).unwrap().debugger.is_none());
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
            (SourceKind::Registry, "registry"),
        ] {
            let src = ExtensionSource {
                kind: kind.clone(),
                path: None,
                member: None,
                url: None,
                id: None,
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

    #[test]
    fn registry_source_roundtrips_and_old_files_still_parse() {
        let src = ExtensionSource {
            kind: SourceKind::Registry,
            path: None,
            member: None,
            url: Some("https://example.invalid/registry.json".into()),
            id: Some("unicorns.rust-lang".into()),
        };
        let out = toml::to_string(&src).unwrap();
        assert!(out.contains("kind = \"registry\""), "{out}");
        let back: ExtensionSource = toml::from_str(&out).unwrap();
        assert_eq!(back.kind, SourceKind::Registry);
        assert_eq!(back.url, src.url);
        assert_eq!(back.id.as_deref(), Some("unicorns.rust-lang"));

        // source.toml written before `id` existed.
        let old: ExtensionSource =
            toml::from_str("kind = \"zip\"\npath = \"/a.zip\"\nmember = \"m\"\n").unwrap();
        assert_eq!(old.kind, SourceKind::Zip);
        assert!(old.id.is_none());
        // `id` is omitted when unset.
        let s = toml::to_string(&old).unwrap();
        assert!(!s.contains("id ="), "{s}");
    }
}
