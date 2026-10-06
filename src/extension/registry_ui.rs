//! "Browse registry" section of the extension picker.
//!
//! Loads the remote index in the background, lists its modules and installs or
//! updates them. Before replacing a module that is already loaded, the app must
//! unload its plugin (releasing the DLL on Windows): requests sit in `pending`
//! until the app calls [`RegistryBrowser::take_unload_requests`], and only then
//! is the download started.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use super::registry::ExtensionRegistry;
use super::remote_registry::{
    self, module_action, ModuleAction, RegistryIndex, RegistryInstallStatus, RegistryModule,
};

#[derive(Debug, Clone, PartialEq)]
enum IndexState {
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

struct PendingInstall {
    module: RegistryModule,
    /// Languages of the installed version, whose plugin must be unloaded first.
    unload_languages: Vec<String>,
}

pub struct RegistryBrowser {
    pub search: String,
    /// URL the current index was requested from.
    loaded_url: String,
    state: IndexState,
    index_rx: Option<Receiver<Result<RegistryIndex, String>>>,
    /// Waiting for the app to unload the module's plugin.
    pending: Vec<PendingInstall>,
    /// Plugin unloaded; the download starts on the next frame.
    ready: Vec<RegistryModule>,
    jobs: HashMap<String, Receiver<RegistryInstallStatus>>,
    statuses: HashMap<String, RegistryInstallStatus>,
}

impl Default for RegistryBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryBrowser {
    pub fn new() -> Self {
        Self {
            search: String::new(),
            loaded_url: String::new(),
            state: IndexState::Idle,
            index_rx: None,
            pending: Vec::new(),
            ready: Vec::new(),
            jobs: HashMap::new(),
            statuses: HashMap::new(),
        }
    }

    /// (Re)load the index from `url` in the background.
    pub fn refresh(&mut self, url: &str, ctx: &egui::Context) {
        self.loaded_url = url.to_string();
        self.state = IndexState::Loading;
        self.index_rx = Some(remote_registry::start_fetch_index(
            url.to_string(),
            Some(ctx.clone()),
        ));
    }

    /// True while `id` is queued, downloading or installing.
    pub fn is_busy(&self, id: &str) -> bool {
        self.jobs.contains_key(id)
            || self.pending.iter().any(|p| p.module.id == id)
            || self.ready.iter().any(|m| m.id == id)
    }

    /// Queue an install/update of `module`. `unload_languages` are the
    /// languages of the currently installed version (empty for a fresh install).
    pub fn request_install(&mut self, module: RegistryModule, unload_languages: Vec<String>) {
        if self.is_busy(&module.id) {
            return;
        }
        self.statuses
            .insert(module.id.clone(), RegistryInstallStatus::Downloading);
        self.pending.push(PendingInstall {
            module,
            unload_languages,
        });
    }

    /// Called by the app each frame: returns the language lists whose plugins
    /// must be unloaded now; the matching installs start on the next frame.
    pub fn take_unload_requests(&mut self) -> Vec<Vec<String>> {
        self.pending
            .drain(..)
            .map(|p| {
                self.ready.push(p.module);
                p.unload_languages
            })
            .filter(|langs| !langs.is_empty())
            .collect()
    }

    /// Poll the index fetch and install jobs. Returns true when an install
    /// finished, so the app reloads plugins (also after a failure, to reload a
    /// plugin that was unloaded for the update).
    pub fn poll(
        &mut self,
        registry: &mut ExtensionRegistry,
        url: &str,
        ctx: &egui::Context,
    ) -> bool {
        let url = url.trim();
        if url.is_empty() {
            if !self.loaded_url.is_empty() {
                self.loaded_url.clear();
                self.state = IndexState::Idle;
                self.index_rx = None;
                registry.remote_index = None;
            }
        } else if self.loaded_url != url || self.state == IndexState::Idle {
            // First open, or the URL changed in the settings.
            registry.remote_index = None;
            self.refresh(url, ctx);
        }

        if let Some(rx) = &self.index_rx {
            if let Ok(res) = rx.try_recv() {
                self.index_rx = None;
                match res {
                    Ok(index) => {
                        registry.remote_index = Some(index);
                        registry.check_updates();
                        self.state = IndexState::Loaded;
                    }
                    Err(e) => {
                        log::warn!("module registry: {e}");
                        self.state = IndexState::Failed(e);
                    }
                }
            }
        }

        for module in self.ready.drain(..) {
            let rx = remote_registry::start_install(
                self.loaded_url.clone(),
                module.clone(),
                registry.extensions_dir.clone(),
                Some(ctx.clone()),
            );
            self.jobs.insert(module.id, rx);
        }

        let mut finished = Vec::new();
        for (id, rx) in &self.jobs {
            while let Ok(status) = rx.try_recv() {
                let done = matches!(
                    status,
                    RegistryInstallStatus::Done { .. } | RegistryInstallStatus::Failed(_)
                );
                self.statuses.insert(id.clone(), status);
                if done {
                    finished.push(id.clone());
                    break;
                }
            }
        }
        for id in &finished {
            self.jobs.remove(id);
        }
        if !finished.is_empty() {
            registry.load_installed();
            registry.check_updates();
        }
        !finished.is_empty()
    }

    /// Inline status for module `id`: text and whether it is an error.
    pub fn status_text(&self, id: &str) -> Option<(String, egui::Color32)> {
        let gray = egui::Color32::from_gray(170);
        let (text, color) = match self.statuses.get(id)? {
            RegistryInstallStatus::Downloading if self.jobs.contains_key(id) => {
                ("Downloading…".to_string(), gray)
            }
            RegistryInstallStatus::Downloading => ("Queued…".to_string(), gray),
            RegistryInstallStatus::Installing => ("Installing…".to_string(), gray),
            RegistryInstallStatus::InstallingDep(step) => (format!("↳ {step}"), gray),
            RegistryInstallStatus::Done { warnings } if warnings.is_empty() => (
                "✓ Installed".to_string(),
                egui::Color32::from_rgb(100, 200, 100),
            ),
            RegistryInstallStatus::Done { warnings } => (
                format!("✓ Installed with warnings: {}", warnings.join("; ")),
                egui::Color32::from_rgb(255, 200, 60),
            ),
            RegistryInstallStatus::Failed(e) => {
                (format!("✗ {e}"), egui::Color32::from_rgb(240, 80, 80))
            }
        };
        Some((text, color))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, registry: &ExtensionRegistry, url: &str) {
        if url.trim().is_empty() {
            ui.label(
                egui::RichText::new(
                    "The module registry is disabled.\n\
                     Set a registry URL in Settings → Extensions to browse modules.",
                )
                .size(11.0)
                .color(egui::Color32::GRAY),
            );
            return;
        }

        let loading = self.state == IndexState::Loading;
        ui.horizontal(|ui| {
            // Button first, right to left, so the field fills exactly what is
            // left: a row wider than the sidebar widens it every frame.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!loading, egui::Button::new("⟳ Refresh"))
                    .clicked()
                {
                    self.refresh(url.trim(), ui.ctx());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search modules…")
                        .desired_width(f32::INFINITY),
                );
            });
        });

        let index = registry.remote_index.as_ref();
        ui.horizontal(|ui| match &self.state {
            IndexState::Idle | IndexState::Loading => {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Loading module registry…")
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );
            }
            IndexState::Failed(e) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("✗ {e}"))
                            .size(11.0)
                            .color(egui::Color32::from_rgb(240, 80, 80)),
                    )
                    .wrap(),
                );
            }
            IndexState::Loaded => {
                let (count, release) = index
                    .map(|i| (i.modules.len(), i.release.as_str()))
                    .unwrap_or((0, ""));
                let release = if release.is_empty() {
                    String::new()
                } else {
                    format!(" · {release}")
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{count} module(s){release} · {}",
                        remote_registry::platform_key()
                    ))
                    .size(11.0)
                    .color(egui::Color32::GRAY),
                );
            }
        });

        let Some(index) = index else { return };
        ui.add_space(4.0);

        let query = self.search.to_lowercase();
        let mut requested: Option<(RegistryModule, Vec<String>)> = None;
        let mut any = false;
        for module in index.modules.iter().filter(|m| m.matches(&query)) {
            any = true;
            let installed = registry
                .installed
                .iter()
                .find(|e| e.manifest.extension.id == module.id);
            let action = module_action(
                module,
                installed.map(|e| e.manifest.extension.version.as_str()),
            );
            let busy = self.is_busy(&module.id);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&module.name)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new(format!("v{}", module.version))
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if busy {
                            ui.spinner();
                            return;
                        }
                        let clicked = match &action {
                            ModuleAction::Install => ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Install")
                                            .small()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 120, 212)),
                                )
                                .clicked(),
                            ModuleAction::Update(v) => ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(format!("Update to {v}"))
                                            .small()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 140, 80)),
                                )
                                .clicked(),
                            ModuleAction::Installed => {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(egui::RichText::new("Installed").small()),
                                );
                                false
                            }
                            ModuleAction::NotAvailable => {
                                ui.label(
                                    egui::RichText::new("Not available for this platform")
                                        .small()
                                        .color(egui::Color32::from_gray(120)),
                                );
                                false
                            }
                        };
                        if clicked {
                            let langs = installed
                                .map(|e| e.manifest.capabilities.languages.clone())
                                .unwrap_or_default();
                            requested = Some((module.clone(), langs));
                        }
                    });
                });

                if !module.description.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&module.description)
                                .size(11.0)
                                .color(egui::Color32::from_gray(160)),
                        )
                        .wrap(),
                    );
                }

                let mut details = Vec::new();
                if !module.languages.is_empty() {
                    details.push(format!("Languages: {}", module.languages.join(", ")));
                }
                if let Some(lsp) = module.lsp_server.as_deref().filter(|s| !s.is_empty()) {
                    details.push(format!("LSP: {lsp}"));
                }
                if !details.is_empty() {
                    ui.label(
                        egui::RichText::new(details.join(" · "))
                            .size(11.0)
                            .color(egui::Color32::from_gray(120)),
                    );
                }

                if let Some((text, color)) = self.status_text(&module.id) {
                    ui.add(egui::Label::new(egui::RichText::new(text).small().color(color)).wrap());
                }
            });
        }

        if !any {
            ui.label(
                egui::RichText::new("No modules match your search.")
                    .size(11.0)
                    .color(egui::Color32::from_gray(140)),
            );
        }

        if let Some((module, langs)) = requested {
            self.request_install(module, langs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(id: &str) -> RegistryModule {
        remote_registry::parse_index(&format!(
            r#"{{"schema":1,"modules":[{{"id":"{id}","dir":"d","name":"N","version":"1.0.0"}}]}}"#
        ))
        .unwrap()
        .modules
        .remove(0)
    }

    #[test]
    fn install_requests_wait_for_unload_then_start() {
        let mut b = RegistryBrowser::new();
        b.request_install(module("a.fresh"), vec![]);
        b.request_install(module("a.update"), vec!["rs".into()]);
        // Duplicate requests are ignored while busy.
        b.request_install(module("a.update"), vec!["rs".into()]);
        assert!(b.is_busy("a.fresh") && b.is_busy("a.update"));
        assert_eq!(
            b.status_text("a.fresh").unwrap().0,
            "Queued…",
            "not started before the app unloads plugins"
        );

        let unloads = b.take_unload_requests();
        assert_eq!(unloads, vec![vec!["rs".to_string()]]);
        assert_eq!(b.ready.len(), 2);
        assert!(b.pending.is_empty());
        assert!(b.take_unload_requests().is_empty());
        assert!(b.is_busy("a.update"));
        assert!(!b.is_busy("other"));
    }

    #[test]
    fn status_text_reports_errors_and_warnings() {
        let mut b = RegistryBrowser::new();
        assert!(b.status_text("x").is_none());
        b.statuses
            .insert("x".into(), RegistryInstallStatus::Failed("boom".into()));
        assert_eq!(b.status_text("x").unwrap().0, "✗ boom");
        b.statuses.insert(
            "x".into(),
            RegistryInstallStatus::Done {
                warnings: vec!["npm missing".into()],
            },
        );
        assert!(b.status_text("x").unwrap().0.contains("npm missing"));
        b.statuses
            .insert("x".into(), RegistryInstallStatus::Done { warnings: vec![] });
        assert_eq!(b.status_text("x").unwrap().0, "✓ Installed");
    }

    #[test]
    fn empty_url_disables_the_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = ExtensionRegistry::new_in(tmp.path().to_path_buf());
        let mut b = RegistryBrowser::new();
        let ctx = egui::Context::default();
        assert!(!b.poll(&mut reg, "  ", &ctx));
        assert_eq!(b.state, IndexState::Idle);
        assert!(b.index_rx.is_none(), "no fetch without a URL");
    }
}
