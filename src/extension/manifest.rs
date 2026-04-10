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
