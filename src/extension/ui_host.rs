//! Interfaces contributed by extension modules (`[[panels]]` in the manifest).
//!
//! A module draws nothing itself: its library returns a *view*, a JSON tree of
//! widgets (`ui_view_ffi(panel_id)`), which this host renders with egui. User
//! input goes back as JSON events (`ui_event_ffi(panel_id, event)`), and the
//! module answers with *actions* for the IDE (toast, run a command in a
//! terminal, open a page…). Views are fetched again after each event and every
//! `poll_ms` while the panel is shown, so a module can report the progress of
//! work it runs on its own threads.
//!
//! View format:
//!
//! ```json
//! { "poll_ms": 1000,
//!   "actions": [ { "type": "toast", "text": "Pulled nginx" } ],
//!   "children": [
//!     { "type": "heading", "text": "Images" },
//!     { "type": "row", "children": [
//!         { "type": "input", "id": "image", "hint": "nginx:latest", "submit": "pull" },
//!         { "type": "button", "id": "pull", "label": "Pull", "icon": "download" } ] },
//!     { "type": "list", "items": [ { "id": "sha256:…", "title": "nginx:latest",
//!         "subtitle": "187 MB", "actions": [ { "id": "remove", "label": "Remove",
//!         "style": "danger", "confirm": "Remove nginx:latest?" } ] } ] } ] }
//! ```
//!
//! Events: `{"type":"click","id":"remove","row":"sha256:…","inputs":{…}}`,
//! `{"type":"submit","id":"image","value":"nginx","inputs":{…}}` (Enter in an
//! input) and `{"type":"toggle","id":…,"checked":true,"inputs":{…}}`. `inputs`
//! holds the current text of every input of the panel.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::ui::theme::Palette;

/// Default refresh interval of a shown panel whose view sets no `poll_ms`.
const DEFAULT_POLL: Duration = Duration::from_millis(1000);

/// Where views and events go: the plugins declaring the panels.
pub trait UiBackend {
    fn view(&self, panel_id: &str) -> Option<String>;
    fn event(&self, panel_id: &str, event: &str) -> Option<String>;
}

impl UiBackend for crate::plugin::manager::PluginManager {
    fn view(&self, panel_id: &str) -> Option<String> {
        self.ui_view(panel_id)
    }
    fn event(&self, panel_id: &str, event: &str) -> Option<String> {
        self.ui_event(panel_id, event)
    }
}

// ── View model ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct View {
    #[serde(default)]
    pub poll_ms: Option<u64>,
    #[serde(default)]
    pub children: Vec<Node>,
    /// Actions run once, when the view is received (e.g. a toast when a
    /// background job finished).
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Heading {
        text: String,
    },
    Text {
        text: String,
        #[serde(default)]
        style: TextStyle,
    },
    Button(ButtonSpec),
    Input {
        id: String,
        #[serde(default)]
        hint: String,
        #[serde(default)]
        value: String,
        /// The input shows `value` again whenever this changes (e.g. to
        /// clear it after a submit).
        #[serde(default)]
        revision: u64,
        /// Button id clicked by Enter.
        #[serde(default)]
        submit: Option<String>,
    },
    Checkbox {
        id: String,
        label: String,
        #[serde(default)]
        checked: bool,
    },
    Row {
        #[serde(default)]
        children: Vec<Node>,
    },
    Group {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        children: Vec<Node>,
    },
    Collapsing {
        id: String,
        title: String,
        #[serde(default)]
        open: bool,
        #[serde(default)]
        children: Vec<Node>,
    },
    List {
        #[serde(default)]
        items: Vec<ListItem>,
        #[serde(default)]
        empty: Option<String>,
    },
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<TableRow>,
        #[serde(default)]
        empty: Option<String>,
    },
    /// Monospace text that can be selected and copied, scrolled to its end
    /// (command output, logs).
    Log {
        text: String,
        /// Height of the scrolled area in points (default 360).
        #[serde(default)]
        height: Option<f32>,
    },
    Separator,
    Spinner {
        #[serde(default)]
        text: Option<String>,
    },
    Space {
        #[serde(default)]
        size: Option<f32>,
    },
    /// Node types of newer IDEs are skipped.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    #[default]
    Normal,
    Muted,
    Strong,
    Code,
    Error,
    Warning,
    Success,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ButtonSpec {
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// Phosphor icon name (see [`icon_glyph`]).
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub style: ButtonStyle,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub tooltip: Option<String>,
    /// Ask this question before sending the click.
    #[serde(default)]
    pub confirm: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
    Danger,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct ListItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub actions: Vec<ButtonSpec>,
}

#[derive(Debug, Deserialize)]
pub struct TableRow {
    pub id: String,
    #[serde(default)]
    pub cells: Vec<String>,
    #[serde(default)]
    pub actions: Vec<ButtonSpec>,
}

/// What a module asks the IDE to do.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Toast {
        text: String,
    },
    /// Run `command` in a new terminal tab (in the workspace).
    Terminal {
        command: String,
    },
    /// Show a module panel: a page opens as an editor tab, a sidebar panel
    /// is selected in the activity bar.
    OpenPanel {
        panel: String,
    },
    OpenUrl {
        url: String,
    },
    OpenFile {
        path: String,
    },
    #[serde(other)]
    Unknown,
}

/// Phosphor glyph for an icon name; other text is shown as is.
pub fn icon_glyph(name: &str) -> &str {
    use egui_phosphor::regular as ph;
    match name {
        "cube" => ph::CUBE,
        "package" => ph::PACKAGE,
        "container" | "shipping-container" => ph::SHIPPING_CONTAINER,
        "stack" => ph::STACK,
        "database" => ph::DATABASE,
        "hard-drives" => ph::HARD_DRIVES,
        "cloud" => ph::CLOUD,
        "globe" => ph::GLOBE,
        "terminal" => ph::TERMINAL_WINDOW,
        "gear" => ph::GEAR,
        "list" => ph::LIST,
        "puzzle" => ph::PUZZLE_PIECE,
        "play" => ph::PLAY,
        "stop" => ph::STOP,
        "trash" => ph::TRASH,
        "refresh" => ph::ARROWS_CLOCKWISE,
        "download" => ph::DOWNLOAD_SIMPLE,
        "broom" => ph::BROOM,
        "info" => ph::INFO,
        "warning" => ph::WARNING,
        "check" => ph::CHECK,
        "close" => ph::X,
        "plus" => ph::PLUS,
        "search" => ph::MAGNIFYING_GLASS,
        "external" => ph::ARROW_SQUARE_OUT,
        "copy" => ph::COPY,
        "folder" => ph::FOLDER,
        "file" => ph::FILE,
        "code" => ph::CODE,
        "bug" => ph::BUG,
        "rocket" => ph::ROCKET,
        "cpu" => ph::CPU,
        other => other,
    }
}

// ── Host ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct PanelState {
    view: Option<View>,
    error: Option<String>,
    fetched: Option<Instant>,
    /// Text of each input, with the `revision` it was last reset from.
    inputs: HashMap<String, (String, u64)>,
    /// Click waiting for the user to confirm: (event, question).
    confirm: Option<(serde_json::Value, String)>,
}

/// Renders module panels and relays their events.
#[derive(Default)]
pub struct UiHost {
    panels: HashMap<String, PanelState>,
    actions: Vec<Action>,
}

/// Input gathered while drawing a view.
struct Draw<'a> {
    palette: Palette,
    inputs: &'a mut HashMap<String, (String, u64)>,
    events: Vec<serde_json::Value>,
    confirm: Option<(serde_json::Value, String)>,
}

impl UiHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every panel's state (modules were reloaded).
    pub fn reset(&mut self) {
        self.panels.clear();
    }

    /// Actions requested by modules since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Draw panel `panel_id` of a module, fetching its view when due.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: &str,
        backend: &dyn UiBackend,
        palette: Palette,
    ) {
        let state = self.panels.entry(panel_id.to_string()).or_default();
        let poll = state
            .view
            .as_ref()
            .and_then(|v| v.poll_ms)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_POLL);
        if state.fetched.is_none_or(|t| t.elapsed() >= poll) {
            Self::fetch(state, &mut self.actions, panel_id, backend);
        }
        ui.ctx().request_repaint_after(poll);

        let state = self.panels.get_mut(panel_id).expect("inserted above");
        let mut draw = Draw {
            palette,
            inputs: &mut state.inputs,
            events: Vec::new(),
            confirm: None,
        };
        if let Some(e) = &state.error {
            ui.label(egui::RichText::new(e).color(palette.error));
        }
        if let Some(view) = &state.view {
            draw_nodes(ui, &view.children, &mut draw);
        }
        let Draw {
            mut events,
            confirm,
            ..
        } = draw;
        if confirm.is_some() {
            state.confirm = confirm;
        }

        if let Some((event, question)) = state.confirm.clone() {
            match confirm_dialog(ui.ctx(), panel_id, &question, palette) {
                Some(true) => {
                    state.confirm = None;
                    events.push(event);
                }
                Some(false) => state.confirm = None,
                None => {}
            }
        }

        for mut event in events {
            let inputs: serde_json::Map<String, serde_json::Value> = state
                .inputs
                .iter()
                .map(|(k, (v, _))| (k.clone(), v.clone().into()))
                .collect();
            event["inputs"] = inputs.into();
            if let Some(reply) = backend.event(panel_id, &event.to_string()) {
                match serde_json::from_str::<Vec<Action>>(&reply) {
                    Ok(actions) => self.actions.extend(actions),
                    Err(e) => log::warn!("{panel_id}: invalid actions: {e}"),
                }
            }
            // Show the outcome right away.
            Self::fetch(state, &mut self.actions, panel_id, backend);
            ui.ctx().request_repaint();
        }
    }

    fn fetch(
        state: &mut PanelState,
        actions: &mut Vec<Action>,
        panel_id: &str,
        backend: &dyn UiBackend,
    ) {
        state.fetched = Some(Instant::now());
        match backend
            .view(panel_id)
            .map(|j| serde_json::from_str::<View>(&j))
        {
            Some(Ok(mut view)) => {
                actions.append(&mut view.actions);
                state.view = Some(view);
                state.error = None;
            }
            Some(Err(e)) => state.error = Some(format!("Invalid view from the module: {e}")),
            None => state.error = Some("The module returned no view.".into()),
        }
    }
}

fn click_event(id: &str, row: Option<&str>) -> serde_json::Value {
    let mut e = serde_json::json!({ "type": "click", "id": id });
    if let Some(row) = row {
        e["row"] = row.into();
    }
    e
}

fn draw_nodes(ui: &mut egui::Ui, nodes: &[Node], d: &mut Draw) {
    for node in nodes {
        draw_node(ui, node, d);
    }
}

fn draw_node(ui: &mut egui::Ui, node: &Node, d: &mut Draw) {
    let p = d.palette;
    match node {
        Node::Heading { text } => {
            ui.label(egui::RichText::new(text).strong().size(14.0).color(p.text));
        }
        Node::Text { text, style } => {
            let t = egui::RichText::new(text);
            let t = match style {
                TextStyle::Muted => t.color(p.text_muted).size(11.0),
                TextStyle::Strong => t.strong().color(p.text),
                TextStyle::Code => t.monospace().color(p.text),
                TextStyle::Error => t.color(p.error),
                TextStyle::Warning => t.color(p.warning),
                TextStyle::Success => t.color(p.success),
                TextStyle::Normal | TextStyle::Unknown => t.color(p.text),
            };
            ui.add(egui::Label::new(t).wrap());
        }
        Node::Button(b) => draw_button(ui, b, None, d),
        Node::Input {
            id,
            hint,
            value,
            revision,
            submit,
        } => {
            let entry = d
                .inputs
                .entry(id.clone())
                .or_insert_with(|| (value.clone(), *revision));
            if entry.1 != *revision {
                *entry = (value.clone(), *revision);
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut entry.0)
                    .hint_text(hint.as_str())
                    .desired_width(ui.available_width().clamp(80.0, 320.0)),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                d.events.push(serde_json::json!({
                    "type": "submit", "id": id, "value": entry.0.clone(),
                }));
                if let Some(button) = submit {
                    d.events.push(click_event(button, None));
                }
            }
        }
        Node::Checkbox { id, label, checked } => {
            let mut value = *checked;
            if ui.checkbox(&mut value, label.as_str()).changed() {
                d.events.push(serde_json::json!({
                    "type": "toggle", "id": id, "checked": value,
                }));
            }
        }
        Node::Row { children } => {
            ui.horizontal_wrapped(|ui| draw_nodes(ui, children, d));
        }
        Node::Group { title, children } => {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(t) = title {
                    ui.label(egui::RichText::new(t).strong().color(p.text));
                }
                draw_nodes(ui, children, d);
            });
        }
        Node::Collapsing {
            id,
            title,
            open,
            children,
        } => {
            egui::CollapsingHeader::new(title.as_str())
                .id_salt(("ext-ui", id.as_str()))
                .default_open(*open)
                .show(ui, |ui| draw_nodes(ui, children, d));
        }
        Node::List { items, empty } => {
            if items.is_empty() {
                if let Some(e) = empty {
                    ui.label(egui::RichText::new(e).color(p.text_muted).size(11.0));
                }
            }
            for item in items {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(egui::RichText::new(&item.title).strong().color(p.text))
                            .truncate(),
                    );
                    if let Some(s) = &item.subtitle {
                        ui.add(
                            egui::Label::new(egui::RichText::new(s).size(11.0).color(p.text_muted))
                                .truncate(),
                        );
                    }
                    if let Some(s) = &item.detail {
                        ui.add(
                            egui::Label::new(egui::RichText::new(s).size(11.0).color(p.text_faint))
                                .wrap(),
                        );
                    }
                    if !item.actions.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for b in &item.actions {
                                draw_button(ui, b, Some(&item.id), d);
                            }
                        });
                    }
                });
            }
        }
        Node::Table {
            columns,
            rows,
            empty,
        } => {
            if rows.is_empty() {
                if let Some(e) = empty {
                    ui.label(egui::RichText::new(e).color(p.text_muted).size(11.0));
                }
                return;
            }
            egui::ScrollArea::horizontal()
                .id_salt(("ext-ui-table", columns.join("|")))
                .show(ui, |ui| {
                    egui::Grid::new(("ext-ui-grid", columns.join("|")))
                        .striped(true)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            for c in columns {
                                ui.label(egui::RichText::new(c).strong().color(p.text_muted));
                            }
                            if rows.iter().any(|r| !r.actions.is_empty()) {
                                ui.label("");
                            }
                            ui.end_row();
                            for row in rows {
                                for cell in &row.cells {
                                    ui.label(egui::RichText::new(cell).color(p.text));
                                }
                                if !row.actions.is_empty() {
                                    ui.horizontal(|ui| {
                                        for b in &row.actions {
                                            draw_button(ui, b, Some(&row.id), d);
                                        }
                                    });
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        Node::Log { text, height } => {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("ext-ui-log")
                    .max_height(height.unwrap_or(360.0))
                    .stick_to_bottom(true)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        // A `&str` buffer: selectable and copyable, not editable.
                        ui.add(
                            egui::TextEdit::multiline(&mut text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .frame(false),
                        );
                    });
            });
        }
        Node::Separator => {
            ui.separator();
        }
        Node::Spinner { text } => {
            ui.horizontal(|ui| {
                ui.spinner();
                if let Some(t) = text {
                    ui.label(egui::RichText::new(t).color(p.text_muted).size(11.0));
                }
            });
        }
        Node::Space { size } => ui.add_space(size.unwrap_or(8.0)),
        Node::Unknown => {}
    }
}

fn draw_button(ui: &mut egui::Ui, b: &ButtonSpec, row: Option<&str>, d: &mut Draw) {
    let p = d.palette;
    let text = match (&b.icon, b.label.is_empty()) {
        (Some(icon), true) => icon_glyph(icon).to_string(),
        (Some(icon), false) => format!("{} {}", icon_glyph(icon), b.label),
        (None, _) => b.label.clone(),
    };
    let (fill, color) = match b.style {
        ButtonStyle::Primary => (Some(p.accent), p.on_accent),
        ButtonStyle::Danger => (Some(p.error), egui::Color32::WHITE),
        ButtonStyle::Default | ButtonStyle::Unknown => (None, p.text),
    };
    let mut button = egui::Button::new(egui::RichText::new(text).color(color));
    if let Some(fill) = fill {
        button = button.fill(fill);
    }
    let mut resp = ui.add_enabled(b.enabled, button);
    if let Some(t) = &b.tooltip {
        resp = resp.on_hover_text(t);
    }
    if resp.clicked() {
        let event = click_event(&b.id, row);
        match &b.confirm {
            Some(question) => d.confirm = Some((event, question.clone())),
            None => d.events.push(event),
        }
    }
}

/// Some(true) confirmed, Some(false) cancelled, None still open.
fn confirm_dialog(
    ctx: &egui::Context,
    panel_id: &str,
    question: &str,
    palette: Palette,
) -> Option<bool> {
    let mut answer = None;
    egui::Window::new("Confirm")
        .id(egui::Id::new(("ext-ui-confirm", panel_id)))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(question).color(palette.text));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
                let ok = egui::Button::new(egui::RichText::new("Confirm").color(palette.on_accent))
                    .fill(palette.accent);
                if ui.add(ok).clicked() {
                    answer = Some(true);
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, RawInput, Rect};
    use std::cell::RefCell;

    /// Module stand-in: serves `view` and records the events it receives.
    struct Fake {
        view: RefCell<String>,
        events: RefCell<Vec<serde_json::Value>>,
        reply: Option<String>,
        views: RefCell<usize>,
    }

    impl Fake {
        fn new(view: &str) -> Self {
            Self {
                view: RefCell::new(view.into()),
                events: RefCell::default(),
                reply: None,
                views: RefCell::new(0),
            }
        }
    }

    impl UiBackend for Fake {
        fn view(&self, _panel: &str) -> Option<String> {
            *self.views.borrow_mut() += 1;
            Some(self.view.borrow().clone())
        }
        fn event(&self, _panel: &str, event: &str) -> Option<String> {
            self.events
                .borrow_mut()
                .push(serde_json::from_str(event).unwrap());
            self.reply.clone()
        }
    }

    fn palette() -> Palette {
        Palette::from_theme(&crate::config::Config::default().theme)
    }

    struct Harness {
        ctx: egui::Context,
        host: UiHost,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                host: UiHost::new(),
            }
        }

        fn frame(&mut self, backend: &Fake, events: Vec<Event>) -> egui::FullOutput {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let host = &mut self.host;
            self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    host.show(ui, "p", backend, palette());
                });
            })
        }

        /// Click the widget whose text is `label`.
        fn click(&mut self, backend: &Fake, label: &str) {
            let out = self.frame(backend, vec![]);
            let pos = text_pos(&out, label).unwrap_or_else(|| panic!("no {label:?}"));
            let press = |pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(backend, vec![Event::PointerMoved(pos), press(true)]);
            self.frame(backend, vec![press(false)]);
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<String> {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    fn text_pos(out: &egui::FullOutput, label: &str) -> Option<Pos2> {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.pos + t.galley.rect.center().to_vec2())
            }
            _ => None,
        })
    }

    const VIEW: &str = r#"{"poll_ms": 60000, "children": [
        {"type":"heading","text":"Images"},
        {"type":"text","text":"2 images","style":"muted"},
        {"type":"future_widget","x":1},
        {"type":"row","children":[
            {"type":"input","id":"image","hint":"name","value":"nginx"},
            {"type":"button","id":"pull","label":"Pull"}]},
        {"type":"list","items":[
            {"id":"sha1","title":"redis:7","subtitle":"40 MB",
             "actions":[{"id":"rm","label":"Remove","style":"danger","confirm":"Remove redis:7?"}]}]},
        {"type":"table","columns":["Repo","Size"],"rows":[{"id":"r1","cells":["alpine","8 MB"]}]},
        {"type":"log","text":"line one\nline two"}
    ]}"#;

    #[test]
    fn renders_every_node_and_skips_unknown_ones() {
        let backend = Fake::new(VIEW);
        let mut h = Harness::new();
        let out = h.frame(&backend, vec![]);
        let t = texts(&out);
        for expected in [
            "Images",
            "2 images",
            "nginx",
            "Pull",
            "redis:7",
            "40 MB",
            "Remove",
            "alpine",
            "8 MB",
            "Repo",
            "line one\nline two",
        ] {
            assert!(t.iter().any(|x| x == expected), "{expected} in {t:?}");
        }
    }

    #[test]
    fn click_sends_the_event_with_inputs_then_refetches() {
        let mut backend = Fake::new(VIEW);
        backend.reply = Some(r#"[{"type":"toast","text":"pulling"}]"#.into());
        let mut h = Harness::new();
        h.click(&backend, "Pull");
        let events = backend.events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "click");
        assert_eq!(events[0]["id"], "pull");
        assert_eq!(events[0]["inputs"]["image"], "nginx");
        assert_eq!(
            h.host.take_actions(),
            [Action::Toast {
                text: "pulling".into()
            }]
        );
        assert!(
            *backend.views.borrow() >= 2,
            "view fetched again after the event"
        );
    }

    #[test]
    fn confirm_holds_the_click_until_confirmed() {
        let backend = Fake::new(VIEW);
        let mut h = Harness::new();
        h.click(&backend, "Remove");
        assert!(backend.events.borrow().is_empty(), "waits for confirmation");
        let out = h.frame(&backend, vec![]);
        assert!(texts(&out).iter().any(|t| t == "Remove redis:7?"));
        h.click(&backend, "Confirm");
        let events = backend.events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["id"], "rm");
        assert_eq!(events[0]["row"], "sha1");
    }

    #[test]
    fn view_actions_run_once_and_errors_are_shown() {
        let backend =
            Fake::new(r#"{"actions":[{"type":"terminal","command":"ls"},{"type":"later"}]}"#);
        let mut h = Harness::new();
        h.frame(&backend, vec![]);
        assert_eq!(
            h.host.take_actions(),
            [
                Action::Terminal {
                    command: "ls".into()
                },
                Action::Unknown
            ]
        );
        h.frame(&backend, vec![]);
        assert!(h.host.take_actions().is_empty(), "not polled again yet");

        *backend.view.borrow_mut() = "not json".into();
        h.host.reset();
        let out = h.frame(&backend, vec![]);
        assert!(texts(&out).iter().any(|t| t.starts_with("Invalid view")));
    }

    #[test]
    fn input_revision_resets_the_text() {
        let backend =
            Fake::new(r#"{"children":[{"type":"input","id":"i","value":"a","revision":1}]}"#);
        let mut h = Harness::new();
        h.frame(&backend, vec![]);
        h.host
            .panels
            .get_mut("p")
            .unwrap()
            .inputs
            .get_mut("i")
            .unwrap()
            .0 = "typed".into();
        h.host.panels.get_mut("p").unwrap().fetched = None;
        h.frame(&backend, vec![]);
        assert_eq!(
            h.host.panels["p"].inputs["i"].0, "typed",
            "same revision keeps it"
        );
        *backend.view.borrow_mut() =
            r#"{"children":[{"type":"input","id":"i","value":"","revision":2}]}"#.into();
        h.host.panels.get_mut("p").unwrap().fetched = None;
        h.frame(&backend, vec![]);
        h.frame(&backend, vec![]);
        assert_eq!(
            h.host.panels["p"].inputs["i"].0, "",
            "new revision resets it"
        );
    }

    #[test]
    fn icon_names_map_to_glyphs() {
        assert_eq!(icon_glyph("trash"), egui_phosphor::regular::TRASH);
        assert_eq!(icon_glyph("🐳"), "🐳");
    }
}
