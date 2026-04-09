use super::installer::{InstallJob, InstallStatus, WorkspaceStatus};
use super::manifest::SourceKind;
use super::registry::ExtensionRegistry;

pub struct ExtensionsPanel {
    pub search_query: String,
    pub install_url: String,
    pub install_job: Option<std::sync::mpsc::Receiver<InstallStatus>>,
    pub install_status: InstallStatus,
    pub local_install_job: Option<std::sync::mpsc::Receiver<InstallStatus>>,
    pub local_install_status: InstallStatus,
    pub generate_name: String,
    pub generate_path: String,
    pub template_message: Option<Result<String, String>>,
    // Workspace installer (local)
    pub workspace_path: String,
    pub workspace_job: Option<std::sync::mpsc::Receiver<WorkspaceStatus>>,
    pub workspace_status: WorkspaceStatus,
    pub workspace_log: Vec<String>,
    // Group installer from git
    pub git_group_url: String,
    pub git_group_job: Option<std::sync::mpsc::Receiver<WorkspaceStatus>>,
    pub git_group_status: WorkspaceStatus,
    pub git_group_log: Vec<String>,
    // Update jobs: extension_id → receiver
    pub update_jobs: std::collections::HashMap<String, std::sync::mpsc::Receiver<InstallStatus>>,
    pub update_statuses: std::collections::HashMap<String, InstallStatus>,
    /// Set to true when any installation completes — the app should reload LSP/plugins.
    pub plugins_changed: bool,
}

impl ExtensionsPanel {
    pub fn new() -> Self {
        let default_path = dirs_next::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("extensions")
            .to_string_lossy()
            .to_string();
        Self {
            search_query: String::new(),
            install_url: String::new(),
            install_job: None,
            install_status: InstallStatus::Idle,
            local_install_job: None,
            local_install_status: InstallStatus::Idle,
            generate_name: String::new(),
            generate_path: default_path,
            template_message: None,
            workspace_path: String::new(),
            workspace_job: None,
            workspace_status: WorkspaceStatus::Idle,
            workspace_log: Vec::new(),
            git_group_url: String::new(),
            git_group_job: None,
            git_group_status: WorkspaceStatus::Idle,
            git_group_log: Vec::new(),
            update_jobs: std::collections::HashMap::new(),
            update_statuses: std::collections::HashMap::new(),
            plugins_changed: false,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, registry: &mut ExtensionRegistry) {
        // Poll update jobs
        let finished_ids: Vec<String> = self
            .update_jobs
            .iter()
            .filter_map(|(id, rx)| {
                if let Ok(status) = rx.try_recv() {
                    self.update_statuses.insert(id.clone(), status.clone());
                    if matches!(status, InstallStatus::Done | InstallStatus::Failed(_)) {
                        return Some(id.clone());
                    }
                }
                None
            })
            .collect();
        for id in &finished_ids {
            self.update_jobs.remove(id);
            if matches!(self.update_statuses.get(id), Some(InstallStatus::Done)) {
                registry.load_installed();
                registry.check_updates();
                self.plugins_changed = true;
            }
        }

        // Poll install job
        if let Some(rx) = &self.install_job {
            if let Ok(status) = rx.try_recv() {
                let done = matches!(status, InstallStatus::Done | InstallStatus::Failed(_));
                let success = matches!(status, InstallStatus::Done);
                if success {
                    self.install_url.clear();
                }
                self.install_status = status;
                if done {
                    self.install_job = None;
                    if success {
                        registry.load_installed();
                        self.plugins_changed = true;
                    }
                }
            }
        }

        // Poll workspace install job (local sources)
        if let Some(workspace_rx) = &self.workspace_job {
            let mut finished = false;
            let mut reload = false;
            while let Ok(status) = workspace_rx.try_recv() {
                let msg = workspace_status_to_log(&status);
                if !msg.is_empty() {
                    self.workspace_log.push(msg);
                }
                if matches!(
                    &status,
                    WorkspaceStatus::Done { .. } | WorkspaceStatus::Failed(_)
                ) {
                    finished = true;
                    reload = matches!(&status, WorkspaceStatus::Done { .. });
                }
                self.workspace_status = status;
            }
            if finished {
                self.workspace_job = None;
                if reload {
                    registry.load_installed();
                    self.plugins_changed = true;
                }
            }
        }

        // Poll git group install job
        if let Some(rx) = &self.git_group_job {
            let mut finished = false;
            let mut reload = false;
            while let Ok(status) = rx.try_recv() {
                let msg = workspace_status_to_log(&status);
                if !msg.is_empty() {
                    self.git_group_log.push(msg);
                }
                if matches!(
                    &status,
                    WorkspaceStatus::Done { .. } | WorkspaceStatus::Failed(_)
                ) {
                    finished = true;
                    reload = matches!(&status, WorkspaceStatus::Done { .. });
                }
                self.git_group_status = status;
            }
            if finished {
                self.git_group_job = None;
                if reload {
                    registry.load_installed();
                    self.plugins_changed = true;
                }
            }
        }

        let max_w = ui.available_width();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(max_w);

            // Search bar
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search extensions…")
                        .desired_width(ui.available_width() - 30.0),
                );
            });
            ui.add_space(6.0);

            // ── INSTALLED ────────────────────────────────────────────────────
            ui.collapsing("INSTALLED", |ui| {
                // "Check for updates" button
                ui.horizontal(|ui| {
                    if ui.small_button("⟳ Check for updates").clicked() {
                        registry.check_updates();
                    }
                    let update_count = registry.installed.iter()
                        .filter(|e| e.update_available.is_some())
                        .count();
                    if update_count > 0 {
                        ui.label(
                            egui::RichText::new(format!("{update_count} update(s) available"))
                                .small()
                                .color(egui::Color32::from_rgb(255, 200, 60)),
                        );
                    }
                });
                ui.add_space(4.0);
                let query = self.search_query.to_lowercase();
                let mut to_uninstall: Option<String> = None;

                let has_matches = registry.installed.iter().any(|e| {
                    query.is_empty()
                        || e.manifest.extension.name.to_lowercase().contains(&query)
                        || e.manifest.extension.id.to_lowercase().contains(&query)
                });

                if !has_matches {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("🧩")
                                .size(32.0)
                                .color(egui::Color32::from_gray(100)),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("No extensions installed yet.")
                                .color(egui::Color32::from_gray(140)),
                        );
                        ui.label(
                            egui::RichText::new("Install one from Git or create your own below.")
                                .size(11.0)
                                .color(egui::Color32::from_gray(100)),
                        );
                    });
                    ui.add_space(12.0);
                } else {
                    let ids: Vec<String> = registry
                        .installed
                        .iter()
                        .filter(|e| {
                            query.is_empty()
                                || e.manifest.extension.name.to_lowercase().contains(&query)
                                || e.manifest.extension.id.to_lowercase().contains(&query)
                        })
                        .map(|e| e.manifest.extension.id.clone())
                        .collect();

                    for (i, id) in ids.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        let ext = registry
                            .installed
                            .iter_mut()
                            .find(|e| &e.manifest.extension.id == id);
                        let Some(ext) = ext else { continue };

                        ui.group(|ui| {
                            ui.set_width(ui.available_width());

                            // Name + version row
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&ext.manifest.extension.name)
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "v{}",
                                        ext.manifest.extension.version
                                    ))
                                    .small()
                                    .color(egui::Color32::from_gray(140)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let uninstall_btn = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("Uninstall")
                                                    .color(egui::Color32::from_rgb(240, 80, 80)),
                                            )
                                            .frame(false),
                                        );
                                        if uninstall_btn.clicked() {
                                            to_uninstall = Some(ext.manifest.extension.id.clone());
                                        }
                                    },
                                );
                            });

                            // Description
                            if !ext.manifest.extension.description.is_empty() {
                                ui.label(
                                    egui::RichText::new(&ext.manifest.extension.description)
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(160)),
                                );
                            }

                            // Author + repo row
                            ui.horizontal(|ui| {
                                if !ext.manifest.extension.author.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&ext.manifest.extension.author)
                                            .size(11.0)
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }
                                if !ext.manifest.extension.repository.is_empty() {
                                    ui.hyperlink_to(
                                        egui::RichText::new("repository")
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(100, 160, 220)),
                                        &ext.manifest.extension.repository,
                                    );
                                }
                            });

                            // Update badge + button
                            if let Some(new_ver) = ext.update_available.clone() {
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "↑ v{new_ver} available"
                                        ))
                                        .small()
                                        .color(egui::Color32::from_rgb(255, 200, 60)),
                                    );
                                    let ext_id = ext.manifest.extension.id.clone();
                                    let is_updating = self.update_jobs.contains_key(&ext_id);
                                    let btn = ui.add_enabled(
                                        !is_updating,
                                        egui::Button::new(
                                            egui::RichText::new("Update")
                                                .small()
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(egui::Color32::from_rgb(0, 120, 212)),
                                    );
                                    if is_updating {
                                        ui.spinner();
                                    }
                                    if btn.clicked() {
                                        if let Some(rx) = start_update_job(&ext.source, &registry.extensions_dir) {
                                            self.update_jobs.insert(ext_id.clone(), rx);
                                            self.update_statuses.insert(ext_id, InstallStatus::Building);
                                        }
                                    }
                                });
                                // Show update status if in progress
                                if let Some(status) = self.update_statuses.get(&ext.manifest.extension.id) {
                                    let (txt, color) = match status {
                                        InstallStatus::Cloning => ("Cloning…", egui::Color32::from_gray(180)),
                                        InstallStatus::Building => ("Building…", egui::Color32::from_gray(180)),
                                        InstallStatus::Installing => ("Installing…", egui::Color32::from_gray(180)),
                                        InstallStatus::InstallingDep(s) => (s.as_str(), egui::Color32::from_gray(160)),
                                        InstallStatus::Done => ("✓ Updated!", egui::Color32::from_rgb(100, 200, 100)),
                                        InstallStatus::Failed(e) => (e.as_str(), egui::Color32::from_rgb(240, 80, 80)),
                                        InstallStatus::Idle => ("", egui::Color32::TRANSPARENT),
                                    };
                                    if !txt.is_empty() {
                                        ui.label(egui::RichText::new(txt).small().color(color));
                                    }
                                }
                            }

                            // Enable/disable
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut ext.enabled, "Enabled");
                            });
                        });
                    }
                }

                if let Some(id) = to_uninstall {
                    if let Err(e) = registry.uninstall(&id) {
                        log::error!("Uninstall failed: {e}");
                    }
                }
            });

            ui.add_space(8.0);

            // ── INSTALL FROM GIT ─────────────────────────────────────────────
            ui.collapsing("INSTALL FROM GIT", |ui| {
                ui.label("Install from Git Repository");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.install_url)
                            .hint_text("https://github.com/user/extension")
                            .desired_width(ui.available_width() - 70.0),
                    );
                    let installing = self.install_job.is_some();
                    let install_btn = ui.add_enabled(
                        !installing && !self.install_url.is_empty(),
                        egui::Button::new(
                            egui::RichText::new("Install").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(0, 120, 212)),
                    );
                    if install_btn.clicked() {
                        let rx = InstallJob::start(
                            self.install_url.clone(),
                            ExtensionRegistry::extensions_dir(),
                        );
                        self.install_job = Some(rx);
                        self.install_status = InstallStatus::Cloning;
                    }
                });

                let (status_text, is_error) = match &self.install_status {
                    InstallStatus::Idle => (String::new(), false),
                    InstallStatus::Cloning => ("Cloning…".to_string(), false),
                    InstallStatus::Building => ("Building…".to_string(), false),
                    InstallStatus::Installing => ("Installing…".to_string(), false),
                    InstallStatus::InstallingDep(step) => (format!("↳ {step}"), false),
                    InstallStatus::Done => ("✓ Installed successfully!".to_string(), false),
                    InstallStatus::Failed(e) => (format!("Error: {e}"), true),
                };

                if !status_text.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if self.install_job.is_some() {
                            ui.spinner();
                        }
                        let color = if is_error {
                            egui::Color32::from_rgb(240, 80, 80)
                        } else if matches!(self.install_status, InstallStatus::Done) {
                            egui::Color32::from_rgb(100, 200, 100)
                        } else {
                            egui::Color32::from_gray(180)
                        };
                        ui.label(egui::RichText::new(status_text).color(color));
                    });
                }
            });

            ui.add_space(8.0);

            // ── LOAD FROM LOCAL FOLDER ────────────────────────────────────────
            ui.collapsing("📁 Load from local folder", |ui| {
                ui.set_max_width(ui.available_width());
                ui.label(
                    egui::RichText::new(
                        "Load an extension from a local directory.\nThe folder must contain a manifest.toml.",
                    )
                    .size(11.0)
                    .color(egui::Color32::GRAY),
                );
                ui.add_space(4.0);

                if ui.button("Browse folder…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        let ext_dir = registry.extensions_dir.clone();
                        self.local_install_job = Some(
                            crate::extension::installer::install_from_folder(folder, ext_dir),
                        );
                        self.local_install_status = InstallStatus::Installing;
                    }
                }

                // Poll local install job
                if let Some(rx) = &self.local_install_job {
                    if let Ok(status) = rx.try_recv() {
                        let done = matches!(status, InstallStatus::Done | InstallStatus::Failed(_));
                        self.local_install_status = status.clone();
                        if done {
                            self.local_install_job = None;
                            if matches!(status, InstallStatus::Done) {
                                registry.load_installed();
                                self.plugins_changed = true;
                            }
                        }
                    }
                }

                match &self.local_install_status {
                    InstallStatus::Idle => {}
                    InstallStatus::Building => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Building…");
                        });
                    }
                    InstallStatus::Installing => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Installing…");
                        });
                    }
                    InstallStatus::InstallingDep(step) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("↳ {step}"));
                        });
                    }
                    InstallStatus::Done => {
                        ui.label(
                            egui::RichText::new("✓ Installed!")
                                .color(egui::Color32::from_rgb(100, 200, 100)),
                        );
                    }
                    InstallStatus::Failed(e) => {
                        ui.label(
                            egui::RichText::new(format!("✗ {e}"))
                                .color(egui::Color32::from_rgb(240, 80, 80)),
                        );
                    }
                    _ => {}
                }
            });

            ui.add_space(8.0);

            // ── INSTALL FROM WORKSPACE ────────────────────────────────────────
            ui.collapsing("⚙ BUILD FROM SOURCES", |ui| {
                ui.set_max_width(ui.available_width());
                ui.label(
                    egui::RichText::new(
                        "Point to a Cargo workspace that contains modules with manifest.toml.\n\
                         Writing Unicorns will run cargo build --release and install all modules.",
                    )
                    .size(11.0)
                    .color(egui::Color32::GRAY),
                );
                ui.add_space(4.0);

                let ws_width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.label("Workspace:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workspace_path)
                            .hint_text("/path/to/modules")
                            .desired_width((ws_width - 90.0).max(40.0)),
                    );
                    if ui.button("Browse…").clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            self.workspace_path = folder.to_string_lossy().to_string();
                        }
                    }
                });
                ui.add_space(4.0);

                let is_building = self.workspace_job.is_some();
                ui.horizontal(|ui| {
                    let can_build = !is_building && !self.workspace_path.is_empty();
                    if ui
                        .add_enabled(
                            can_build,
                            egui::Button::new(
                                egui::RichText::new("▶ Build & Install All")
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(0, 140, 80)),
                        )
                        .clicked()
                    {
                        self.workspace_log.clear();
                        self.workspace_status = WorkspaceStatus::Building;
                        let rx = super::installer::install_from_workspace(
                            std::path::PathBuf::from(&self.workspace_path),
                            registry.extensions_dir.clone(),
                        );
                        self.workspace_job = Some(rx);
                    }

                    if is_building {
                        ui.spinner();
                    }

                    if !self.workspace_log.is_empty() && ui.small_button("Clear log").clicked() {
                        self.workspace_log.clear();
                        self.workspace_status = WorkspaceStatus::Idle;
                    }
                });

                // Log output
                if !self.workspace_log.is_empty() {
                    ui.add_space(4.0);
                    let log_width = ui.available_width();
                    show_log_area(ui, &self.workspace_log, log_width, "ws_log_scroll");
                }
            });

            ui.add_space(8.0);

            // ── INSTALL GROUP FROM GIT ────────────────────────────────────────
            ui.collapsing("📦 INSTALL GROUP FROM GIT", |ui| {
                ui.set_max_width(ui.available_width());
                ui.label(
                    egui::RichText::new(
                        "Clone a git repository containing a Cargo workspace of modules.\n\
                         Writing Unicorns will build and install every member that has a manifest.toml.\n\
                         Single-extension repositories are also supported.",
                    )
                    .size(11.0)
                    .color(egui::Color32::GRAY),
                );
                ui.add_space(4.0);

                let section_width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.git_group_url)
                            .hint_text("https://github.com/user/my-modules-workspace")
                            .desired_width((section_width - 110.0).max(40.0)),
                    );
                    let is_running = self.git_group_job.is_some();
                    let can_run = !is_running && !self.git_group_url.is_empty();
                    if ui
                        .add_enabled(
                            can_run,
                            egui::Button::new(
                                egui::RichText::new("Install All").color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(0, 120, 212)),
                        )
                        .clicked()
                    {
                        self.git_group_log.clear();
                        self.git_group_status = WorkspaceStatus::Cloning;
                        let rx = super::installer::install_group_from_git(
                            self.git_group_url.clone(),
                            registry.extensions_dir.clone(),
                        );
                        self.git_group_job = Some(rx);
                    }
                    if is_running {
                        ui.spinner();
                    }
                    if !self.git_group_log.is_empty() && ui.small_button("Clear").clicked() {
                        self.git_group_log.clear();
                        self.git_group_status = WorkspaceStatus::Idle;
                    }
                });

                if !self.git_group_log.is_empty() {
                    ui.add_space(4.0);
                    let log_width = ui.available_width();
                    show_log_area(ui, &self.git_group_log, log_width, "git_group_log_scroll");
                }
            });

            ui.add_space(8.0);
            ui.collapsing("CREATE EXTENSION", |ui| {
                ui.label("Create Extension Template");
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Name:");
                    let w = (ui.available_width() - 10.0).max(40.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.generate_name)
                            .hint_text("my-extension")
                            .desired_width(w),
                    );
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    let w = (ui.available_width() - 10.0).max(40.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.generate_path)
                            .desired_width(w),
                    );
                });
                ui.add_space(4.0);

                if ui
                    .add_enabled(
                        !self.generate_name.is_empty() && !self.generate_path.is_empty(),
                        egui::Button::new("Generate"),
                    )
                    .clicked()
                {
                    let output_dir = std::path::PathBuf::from(&self.generate_path);
                    match super::template::generate_extension_template(
                        &self.generate_name,
                        &output_dir,
                    ) {
                        Ok(()) => {
                            let path = output_dir
                                .join(&self.generate_name)
                                .to_string_lossy()
                                .to_string();
                            self.template_message = Some(Ok(path));
                        }
                        Err(e) => {
                            self.template_message = Some(Err(e.to_string()));
                        }
                    }
                }

                if let Some(result) = &self.template_message {
                    ui.add_space(4.0);
                    match result {
                        Ok(path) => {
                            ui.label(
                                egui::RichText::new(format!("✓ Created at {path}"))
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                        }
                        Err(e) => {
                            ui.label(
                                egui::RichText::new(format!("Error: {e}"))
                                    .color(egui::Color32::from_rgb(240, 80, 80)),
                            );
                        }
                    }
                    if ui.button("Dismiss").clicked() {
                        self.template_message = None;
                    }
                }
            });
        });
    }
}

/// Start a reinstall job based on where the extension was originally installed from.
fn start_update_job(
    source: &Option<super::manifest::ExtensionSource>,
    extensions_dir: &std::path::Path,
) -> Option<std::sync::mpsc::Receiver<InstallStatus>> {
    let source = source.as_ref()?;
    let ext_dir = extensions_dir.to_path_buf();
    match &source.kind {
        SourceKind::Workspace => {
            let ws_path = std::path::PathBuf::from(source.path.as_deref()?);
            let member = source.member.as_deref()?.to_string();
            // Reinstall the single member folder (already built in the workspace target/)
            let folder = ws_path.join(&member);
            Some(super::installer::install_from_folder(folder, ext_dir))
        }
        SourceKind::Folder => {
            let folder = std::path::PathBuf::from(source.path.as_deref()?);
            Some(super::installer::install_from_folder(folder, ext_dir))
        }
        SourceKind::Git => {
            let url = source.url.as_deref()?.to_string();
            Some(super::installer::InstallJob::start(url, ext_dir))
        }
    }
}

impl Default for ExtensionsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn workspace_status_to_log(status: &WorkspaceStatus) -> String {
    match status {
        WorkspaceStatus::Cloning => "🔄 Cloning repository…".to_string(),
        WorkspaceStatus::Building => "⚙ Building workspace…".to_string(),
        WorkspaceStatus::Installing {
            current,
            done,
            total,
        } => {
            format!("📦 [{}/{total}] Installing {current}…", done + 1)
        }
        WorkspaceStatus::InstallingDep { module, step } => {
            format!("  ↳ [{module}] {step}")
        }
        WorkspaceStatus::ModuleFailed { name, reason } => {
            format!("⚠ {name}: {reason}")
        }
        WorkspaceStatus::Done { installed, total } => {
            format!("✓ Done — {installed}/{total} modules installed")
        }
        WorkspaceStatus::Failed(e) => format!("✗ {e}"),
        WorkspaceStatus::Idle => String::new(),
    }
}

/// Render the coloured log area used by both workspace and git-group installers.
/// `log_width` must be captured from `ui.available_width()` *before* entering
/// the frame so the frame cannot cause the sidebar to grow.
fn show_log_area(ui: &mut egui::Ui, lines: &[String], log_width: f32, id: &str) {
    let log_bg = egui::Color32::from_rgb(18, 18, 18);
    egui::Frame::new()
        .fill(log_bg)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            // Constrain the frame itself so it never pushes the sidebar wider.
            ui.set_max_width(log_width);
            egui::ScrollArea::vertical()
                .max_height(120.0)
                // Allow horizontal shrinking so long lines don't expand the panel.
                .auto_shrink([true, false])
                .stick_to_bottom(true)
                .id_salt(id)
                .show(ui, |ui| {
                    ui.set_max_width((log_width - 12.0).max(40.0));
                    ui.style_mut().spacing.item_spacing.y = 2.0;
                    for line in lines {
                        let color = if line.starts_with('✓') || line.contains("Done") {
                            egui::Color32::from_rgb(100, 200, 100)
                        } else if line.starts_with('✗') || line.starts_with('⚠') {
                            egui::Color32::from_rgb(240, 120, 80)
                        } else {
                            egui::Color32::from_gray(180)
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(line)
                                    .size(11.0)
                                    .color(color)
                                    .monospace(),
                            )
                            .wrap(),
                        );
                    }
                });
        });
}
