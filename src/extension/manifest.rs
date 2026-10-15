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
    /// Interfaces the module adds to the IDE (`[[panels]]`): sidebar panels
    /// and pages, drawn from the views the module's library returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelSpec>,
}

/// `[[panels]]` entry: an interface contributed by a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelSpec {
    /// Unique id, passed to the module's `ui_view_ffi` / `ui_event_ffi`.
    pub id: String,
    pub title: String,
    /// Phosphor icon name (`cube`, `package`, `database`…) or any text.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub location: PanelLocation,
}

/// Where a module's panel is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelLocation {
    /// Entry in the activity bar, shown in the sidebar.
    #[default]
    Sidebar,
    /// Editor tab, opened by the module (`open_panel` action) or the
    /// command palette.
    Page,
}

/// Language server of one language, when a module's languages need
/// different servers (`[capabilities.lsp_servers.<language>]`): a command to
/// start, or the address of a server already running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageServer {
    /// Executable started on stdio. Exclusive with `connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// `host:port` of a server already listening (e.g. inside the Godot
    /// editor), reached over TCP. Exclusive with `command`, `args` and
    /// `download`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
    /// `initializationOptions`, instead of the module's `lsp_init_options`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_options: Option<toml::Table>,
    /// Archive with the server's files, like the module's `lsp_download`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<ArchiveDownload>,
}

impl LanguageServer {
    fn validate(&self, language: &str) -> anyhow::Result<()> {
        let section = format!("[capabilities.lsp_servers.{language}]");
        match (&self.command, &self.connect) {
            (Some(_), None) => Ok(()),
            (None, Some(addr)) if self.args.is_empty() && self.download.is_none() => {
                validate_connect(&section, addr)
            }
            (None, Some(_)) => {
                anyhow::bail!("{section}: `args` and `download` cannot be used with `connect`")
            }
            (Some(_), Some(_)) => anyhow::bail!("{section}: set `command` or `connect`, not both"),
            (None, None) => anyhow::bail!("{section}: `command` or `connect` is required"),
        }
    }
}

/// Check the `connect` address of `section`: `host:port`, port 1–65535.
fn validate_connect(section: &str, addr: &str) -> anyhow::Result<()> {
    let valid = addr.rsplit_once(':').is_some_and(|(host, port)| {
        let host_ok = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            Some(v6) => v6.parse::<std::net::Ipv6Addr>().is_ok(),
            None => {
                !host.is_empty()
                    && !host.contains(|c: char| c == ':' || c == '/' || c.is_whitespace())
            }
        };
        host_ok && port.parse::<u16>().is_ok_and(|p| p != 0)
    });
    if !valid {
        anyhow::bail!("{section}: `connect` must be `host:port` (IPv6 in brackets), got `{addr}`");
    }
    Ok(())
}

/// `[debugger]` section: how to start the language's debug adapter (DAP
/// over stdio, over a local TCP port, or started by the language server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebuggerSpec {
    /// VS Code debug `type`s handled (e.g. `["coreclr"]`), matched against
    /// launch configurations.
    #[serde(default)]
    pub types: Vec<String>,
    /// Executable, or candidates tried in order, looked up on `PATH`. With
    /// `transport = "lsp"`: the language server command (`workspace/
    /// executeCommand`) that starts a debug session and returns its port.
    /// Exclusive with `connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<OneOrMany>,
    /// `host:port` of an adapter already running (e.g. inside the Godot
    /// editor): the IDE connects to it and starts nothing. Exclusive with
    /// `command`, `args`, `transport`, `download` and `lsp_resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
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
    pub download: Option<ArchiveDownload>,
    /// With `transport = "lsp"`: language server commands run in order
    /// before starting the session, each completing the launch arguments
    /// (e.g. the main class, then its classpath).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lsp_resolve: Vec<LspResolveStep>,
}

impl DebuggerSpec {
    /// Executables to try, in order (with `transport = "lsp"`: the server
    /// command); none for an adapter the IDE connects to.
    pub fn commands(&self) -> &[String] {
        self.command
            .as_ref()
            .map(OneOrMany::as_slice)
            .unwrap_or(&[])
    }

    fn validate(&self) -> anyhow::Result<()> {
        const SECTION: &str = "[debugger]";
        match (&self.command, &self.connect) {
            (Some(_), None) => Ok(()),
            (None, Some(addr)) => {
                if !self.args.is_empty()
                    || self.transport != DebuggerTransport::Stdio
                    || self.download.is_some()
                    || !self.lsp_resolve.is_empty()
                {
                    anyhow::bail!(
                        "{SECTION}: `args`, `transport`, `download` and `lsp_resolve` cannot be used with `connect`"
                    );
                }
                validate_connect(SECTION, addr)
            }
            (Some(_), Some(_)) => anyhow::bail!("{SECTION}: set `command` or `connect`, not both"),
            (None, None) => anyhow::bail!("{SECTION}: `command` or `connect` is required"),
        }
    }
}

/// One `[[debugger.lsp_resolve]]` step: a `workspace/executeCommand` whose
/// result fills launch arguments. In `args`, strings can use `${file}`,
/// `${fileUri}`, `${workspaceFolder}`, `${workspaceFolderUri}` and
/// `${launch:<key>}` (a launch argument, e.g. one set by an earlier step).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspResolveStep {
    pub command: String,
    #[serde(default)]
    pub args: Vec<toml::Value>,
    /// Skip the step when the launch arguments already have this key (set
    /// by the launch configuration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unless: Option<String>,
    /// The result is a list of objects: take the one whose `pick` field is
    /// the current file, else the first. An empty list is retried while
    /// the server loads the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    /// Fields of the (picked) result object copied into the launch arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge: Vec<String>,
    /// Launch arguments set from the result: one key takes the whole
    /// result, several take the items of a list result in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
}

/// An archive a module downloads on first use (`[debugger.download]`,
/// `[capabilities.lsp_download]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveDownload {
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
    /// The language server starts the session (`command` run with
    /// `workspace/executeCommand` returns the port); the IDE connects to it.
    Lsp,
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
    /// directory name under the extensions dir, and server sections must
    /// name exactly one way to reach the server.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(s)?;
        validate_extension_id(&manifest.extension.id)?;
        for (language, server) in &manifest.capabilities.lsp_servers {
            server.validate(language)?;
        }
        if let Some(debugger) = &manifest.debugger {
            debugger.validate()?;
        }
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
    /// Arguments to pass to the LSP server (e.g. ["--stdio"]). Variables:
    /// `${serverDir}` (where `lsp_download` is unpacked), `${dataDir}` (a
    /// folder for the server's data, one per workspace), `${os}` (`win`,
    /// `linux` or `mac`). A path whose file name contains `*` is replaced
    /// by the matching file (e.g. a jar named after its version).
    #[serde(default)]
    pub lsp_args: Vec<String>,
    /// Archive with the server's files, unpacked in the module's folder
    /// (`${serverDir}`) when the first file of its languages is opened: when
    /// the command or arguments use `${serverDir}`, else when the command is
    /// not on `PATH` (the downloaded `binary` is then the command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_download: Option<ArchiveDownload>,
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
    /// Language server per language, overriding `lsp_server` / `lsp_args`
    /// (e.g. a Docker module: docker-langserver for Dockerfiles,
    /// docker-compose-langserver for Compose files).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub lsp_servers: std::collections::BTreeMap<String, LanguageServer>,
}

impl Capabilities {
    /// Language server command for `language`: its own entry in
    /// `lsp_servers`, else the module's `lsp_server`. `None` for a server
    /// reached with `connect`.
    pub fn lsp_command(&self, language: &str) -> Option<(String, Vec<String>)> {
        match self.lsp_servers.get(language) {
            Some(s) => Some((s.command.clone()?, s.args.clone())),
            None => Some((self.lsp_server.clone()?, self.lsp_args.clone())),
        }
    }

    /// `initializationOptions` for `language`'s server.
    pub fn lsp_init_options_for(&self, language: &str) -> Option<&toml::Table> {
        match self.lsp_servers.get(language) {
            Some(s) => s.init_options.as_ref(),
            None => self.lsp_init_options.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugger_can_connect_to_a_running_adapter() {
        let with = |body: &str| ExtensionManifest::parse(&format!("{FULL}\n[debugger]\n{body}\n"));
        let d = with("types = [\"godot\"]\nconnect = \"127.0.0.1:6006\"")
            .unwrap()
            .debugger
            .unwrap();
        assert_eq!(d.connect.as_deref(), Some("127.0.0.1:6006"));
        assert!(d.commands().is_empty());
        for (bad, fragment) in [
            (
                "command = \"x\"\nconnect = \"127.0.0.1:6006\"",
                "not both",
            ),
            ("types = [\"godot\"]", "is required"),
            (
                "connect = \"127.0.0.1:6006\"\nargs = [\"--x\"]",
                "cannot be used with",
            ),
            (
                "connect = \"127.0.0.1:6006\"\ntransport = \"tcp\"",
                "cannot be used with",
            ),
            (
                "connect = \"127.0.0.1:6006\"\ntransport = \"lsp\"",
                "cannot be used with",
            ),
            ("connect = \"127.0.0.1\"", "must be `host:port`"),
            (
                "connect = \"127.0.0.1:6006\"\n[debugger.download]\nbinary = \"x\"\n[debugger.download.urls]\nwindows-x86_64 = \"https://example.invalid/x.zip\"",
                "cannot be used with",
            ),
            (
                "connect = \"127.0.0.1:6006\"\n[[debugger.lsp_resolve]]\ncommand = \"x\"",
                "cannot be used with",
            ),
        ] {
            let err = with(bad).unwrap_err().to_string();
            assert!(err.contains("[debugger]"), "{bad}: {err}");
            assert!(err.contains(fragment), "{bad}: {err}");
        }
    }

    #[test]
    fn per_language_server_can_connect_instead_of_starting() {
        let with = |entry: &str| {
            ExtensionManifest::parse(&format!(
                "{FULL}\n[capabilities.lsp_servers.pyw]\n{entry}\n"
            ))
        };
        let m = with("connect = \"127.0.0.1:6005\"").unwrap();
        let s = &m.capabilities.lsp_servers["pyw"];
        assert_eq!(s.connect.as_deref(), Some("127.0.0.1:6005"));
        assert_eq!(s.command, None);
        // A connected server has no command to name.
        assert_eq!(m.capabilities.lsp_command("pyw"), None);
        for ok in ["[::1]:6005", "localhost:6005"] {
            with(&format!("connect = \"{ok}\"")).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for (bad, fragment) in [
            ("command = \"x\"\nconnect = \"127.0.0.1:6005\"", "not both"),
            ("args = [\"--stdio\"]", "is required"),
            (
                "connect = \"127.0.0.1:6005\"\nargs = [\"x\"]",
                "cannot be used with",
            ),
            (
                "connect = \"127.0.0.1:6005\"\ndownload = { binary = \"x\", urls = { \"*\" = \"https://example.invalid/x.zip\" } }",
                "cannot be used with",
            ),
            ("connect = \"localhost\"", "must be `host:port`"),
            ("connect = \"127.0.0.1:0\"", "must be `host:port`"),
            ("connect = \":6005\"", "must be `host:port`"),
            ("connect = \"http://127.0.0.1:6005\"", "must be `host:port`"),
            ("connect = \" 127.0.0.1:6005\"", "must be `host:port`"),
            ("connect = \"::1\"", "must be `host:port`"),
        ] {
            let err = with(bad).unwrap_err().to_string();
            assert!(err.contains("[capabilities.lsp_servers.pyw]"), "{bad}: {err}");
            assert!(err.contains(fragment), "{bad}: {err}");
        }
    }

    #[test]
    fn connect_entry_survives_a_toml_round_trip() {
        let m = ExtensionManifest::parse(&format!(
            "{FULL}\n[capabilities.lsp_servers.pyw]\nconnect = \"127.0.0.1:6005\"\n"
        ))
        .unwrap();
        let out = toml::to_string(&m).unwrap();
        let back = ExtensionManifest::parse(&out).unwrap();
        assert_eq!(
            back.capabilities.lsp_servers["pyw"].connect.as_deref(),
            Some("127.0.0.1:6005")
        );
    }

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
    fn per_language_servers_override_the_module_server() {
        let m = ExtensionManifest::parse(&FULL.replace(
            "lsp_args = [\"--stdio\"]",
            "lsp_args = [\"--stdio\"]
lsp_init_options = { a = 1 }
             [capabilities.lsp_servers.pyw]
command = \"other\"
init_options = { b = 2 }",
        ))
        .unwrap();
        let c = &m.capabilities;
        assert_eq!(
            c.lsp_command("py"),
            Some(("pylsp".into(), vec!["--stdio".into()]))
        );
        assert_eq!(c.lsp_command("pyw"), Some(("other".into(), vec![])));
        assert!(c.lsp_init_options_for("py").unwrap().contains_key("a"));
        assert!(c.lsp_init_options_for("pyw").unwrap().contains_key("b"));
        let none = ExtensionManifest::parse(&FULL.replace(
            "lsp_server = \"pylsp\"
",
            "",
        ))
        .unwrap();
        assert_eq!(none.capabilities.lsp_command("py"), None);
    }

    #[test]
    fn parses_panels() {
        let m = ExtensionManifest::parse(&format!(
            "{FULL}
[[panels]]
id = \"a.side\"
title = \"Side\"
icon = \"cube\"

             [[panels]]
id = \"a.page\"
title = \"Page\"
location = \"page\"
"
        ))
        .unwrap();
        assert_eq!(m.panels.len(), 2);
        assert_eq!(m.panels[0].location, PanelLocation::Sidebar);
        assert_eq!(m.panels[0].icon.as_deref(), Some("cube"));
        assert_eq!(m.panels[1].location, PanelLocation::Page);
        assert!(ExtensionManifest::parse(FULL).unwrap().panels.is_empty());
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
        assert_eq!(d.commands(), ["netcoredbg"]);
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
        assert_eq!(d.commands(), ["python", "python3"]);
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
