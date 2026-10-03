use super::CodingUnicorns;
use crate::updater::{UpdateEvent, UpdateState, Updater};

impl CodingUnicorns {
    /// Poll the background updater and show the update dialog when relevant.
    pub(crate) fn update_updater(&mut self, ctx: &egui::Context) {
        if let Some(ev) = self.updater.poll() {
            match ev {
                UpdateEvent::UpToDate => self.toast(format!(
                    "Coding Unicorns {} is up to date",
                    Updater::current_version()
                )),
                UpdateEvent::Available(v) => self.toast(format!("Update available: v{v}")),
                UpdateEvent::Ready => {}
                UpdateEvent::Error(e) => self.toast(format!("Update failed: {e}")),
            }
        }

        if self.updater.dismissed {
            return;
        }
        let info = match &self.updater.state {
            UpdateState::Available(i) | UpdateState::Downloading(i) | UpdateState::Ready(i) => {
                i.clone()
            }
            _ => return,
        };

        let mut install = false;
        let mut later = false;
        let mut skip = false;
        let mut restart = false;
        egui::Window::new("Update available")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Coding Unicorns v{} is available (you have v{}).",
                    info.version,
                    Updater::current_version()
                ));
                if !info.notes.trim().is_empty() {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .max_width(420.0)
                        .show(ui, |ui| {
                            egui_commonmark::CommonMarkViewer::new().show(
                                ui,
                                &mut self.md_cache,
                                &info.notes,
                            );
                        });
                }
                ui.hyperlink_to("Download page", &info.page_url);
                ui.add_space(8.0);
                ui.horizontal(|ui| match &self.updater.state {
                    UpdateState::Available(_) => {
                        install = ui.button("Install").clicked();
                        later = ui.button("Later").clicked();
                        skip = ui.button("Skip this version").clicked();
                    }
                    UpdateState::Downloading(_) => {
                        ui.spinner();
                        ui.label("Downloading…");
                    }
                    UpdateState::Ready(_) => {
                        ui.label("Update ready.");
                        restart = ui.button("Restart now").clicked();
                        later = ui.button("When I quit").clicked();
                    }
                    _ => {}
                });
            });

        if install {
            self.updater.install(ctx);
        }
        if later {
            self.updater.schedule_on_quit();
            self.updater.dismissed = true;
        }
        if skip {
            self.config.skipped_update_version = Some(info.version.to_string());
            self.config.save();
            self.updater.dismissed = true;
        }
        if restart {
            // Goes through the normal close path, so the unsaved-files prompt still applies.
            // The relaunch / installer run happens in `on_exit`.
            self.updater.schedule_restart();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
