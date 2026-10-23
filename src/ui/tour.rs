//! Welcome tour: a few cards shown on first launch (and on demand from the
//! command palette) that spotlight each part of the window and explain how
//! the IDE works.

use crate::config::KeyBindings;
use crate::ui::theme::{Palette, Spacing};
use egui::{Color32, Pos2, Rect, Vec2};

/// Part of the window a step points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// No spotlight: the card sits in the middle of the window.
    Center,
    ActivityBar,
    /// The sidebar showing the Explorer.
    Explorer,
    /// The sidebar showing the Extensions view.
    Extensions,
    Editor,
    Terminal,
    StatusBar,
}

/// Where the tour's targets were drawn this frame, recorded by the layout.
/// A hidden panel leaves its rect `None`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Anchors {
    pub activity_bar: Option<Rect>,
    pub sidebar: Option<Rect>,
    pub editor: Option<Rect>,
    pub terminal: Option<Rect>,
    pub status_bar: Option<Rect>,
}

impl Anchors {
    fn rect(&self, target: Target) -> Option<Rect> {
        match target {
            Target::Center => None,
            Target::ActivityBar => self.activity_bar,
            Target::Explorer | Target::Extensions => self.sidebar,
            Target::Editor => self.editor,
            Target::Terminal => self.terminal,
            Target::StatusBar => self.status_bar,
        }
    }
}

struct Step {
    target: Target,
    title: &'static str,
    body: fn(&KeyBindings) -> String,
}

const STEPS: &[Step] = &[
    Step {
        target: Target::Center,
        title: "Welcome to Coding Unicorns 🦄",
        body: |_| {
            "A lightweight IDE written in Rust.\n\n\
             It ships deliberately bare: no language is built in. Syntax highlighting, \
             language servers (completion, diagnostics, go to definition) and debuggers \
             all come from installable extensions, so the editor only carries what you use.\n\n\
             This short tour shows where everything is. Use → / ← or the buttons below, \
             Esc to skip."
                .to_string()
        },
    },
    Step {
        target: Target::ActivityBar,
        title: "Activity bar",
        body: |kb| {
            format!(
                "Switches the sidebar between its views: Explorer, Search, Git, Extensions, \
                 Run, Outline and Debug. Click the active view again, or press {}, to hide \
                 the sidebar.\n\nThe gear at the bottom opens the Settings.",
                kb.toggle_sidebar.display()
            )
        },
    },
    Step {
        target: Target::Explorer,
        title: "Explorer",
        body: |kb| {
            format!(
                "Start by opening a project: File → Open Folder ({}). The tree shows your \
                 files with their git status; right-click for new file, rename, delete…\n\n\
                 The folder and its open tabs are restored at the next launch.",
                kb.open_folder.display()
            )
        },
    },
    Step {
        target: Target::Extensions,
        title: "Extensions: add your languages",
        body: |_| {
            "This is the key step. Install the module of each language you work with \
             (Rust, Python, TypeScript, C#, Go…) from the online registry.\n\n\
             A module brings the highlighting, the language server and the debugger of its \
             language. Their own dependencies (npm, pip, cargo…) are installed for you."
                .to_string()
        },
    },
    Step {
        target: Target::Editor,
        title: "Editor",
        body: |kb| {
            format!(
                "Files open in tabs. {} splits the editor side by side.\n\n\
                 Multi-cursors: {} selects the next occurrence, Ctrl+Click adds a cursor.\n\
                 With a language module: {} goes to the definition, {} comes back.",
                kb.toggle_split.display(),
                kb.select_next_occurrence.display(),
                kb.goto_definition.display(),
                kb.navigate_back.display()
            )
        },
    },
    Step {
        target: Target::Center,
        title: "Command palette",
        body: |kb| {
            format!(
                "{} finds any file of the project by name. Start the query with:\n\n\
                 >   to run a command (Ctrl+Shift+P opens it directly)\n\
                 @   to jump to a symbol of the current file\n\
                 #   to search symbols in the whole workspace\n\n\
                 {} searches text across the workspace.",
                kb.command_palette.display(),
                kb.format_document.display()
            )
        },
    },
    Step {
        target: Target::Terminal,
        title: "Terminal",
        body: |kb| {
            format!(
                "A real shell, with several tabs. {} shows or hides it.\n\n\
                 Run and debug configurations (launch.toml, or imported from .vscode) \
                 start from the Run view or with {}.",
                kb.toggle_terminal.display(),
                kb.debug_start.display()
            )
        },
    },
    Step {
        target: Target::StatusBar,
        title: "Status bar",
        body: |_| {
            "The current branch and its ahead / behind count, the state of the language \
             server, and the errors and warnings of the project: click them to open the \
             Problems panel."
                .to_string()
        },
    },
    Step {
        target: Target::Center,
        title: "You're all set",
        body: |kb| {
            format!(
                "{} opens a Claude Code panel to work on the project with Claude \
                 (needs the claude CLI).\n\n\
                 {} lists every shortcut, and the Settings ({}) change the theme, the \
                 editor options and the keybindings.\n\n\
                 Found a bug or have an idea? Help → Report an Issue sends it to the \
                 developers. Replay this tour any time from the Help menu.",
                crate::keybinds::platform_label("Ctrl+Shift+I"),
                kb.shortcuts_help.display(),
                kb.settings.display()
            )
        },
    },
];

const CARD_WIDTH: f32 = 380.0;
/// Gap between the spotlight and the card, and margin to the window edges.
const GAP: f32 = 14.0;

pub struct Tour {
    step: Option<usize>,
    /// Size of the card last frame, used to place it before it's laid out.
    card_size: Vec2,
    pub anchors: Anchors,
}

impl Default for Tour {
    fn default() -> Self {
        Self::new()
    }
}

impl Tour {
    pub fn new() -> Self {
        Self {
            step: None,
            card_size: Vec2::new(CARD_WIDTH, 240.0),
            anchors: Anchors::default(),
        }
    }

    pub fn start(&mut self) {
        self.step = Some(0);
    }

    pub fn is_active(&self) -> bool {
        self.step.is_some()
    }

    /// What the current step points at, so the app can show that panel.
    pub fn target(&self) -> Option<Target> {
        self.step.map(|i| STEPS[i].target)
    }

    fn next(&mut self) -> bool {
        match self.step {
            Some(i) if i + 1 < STEPS.len() => {
                self.step = Some(i + 1);
                false
            }
            _ => self.finish(),
        }
    }

    fn back(&mut self) {
        if let Some(i) = self.step {
            self.step = Some(i.saturating_sub(1));
        }
    }

    fn finish(&mut self) -> bool {
        self.step = None;
        true
    }

    /// Arrow keys / Enter / Esc drive the tour. Runs before the rest of the
    /// UI so the editor never sees those keys. True when the tour just ended.
    pub fn handle_keys(&mut self, ctx: &egui::Context) -> bool {
        if !self.is_active() {
            return false;
        }
        let (next, back, skip) = ctx.input_mut(|i| {
            let none = egui::Modifiers::NONE;
            (
                i.consume_key(none, egui::Key::ArrowRight) | i.consume_key(none, egui::Key::Enter),
                i.consume_key(none, egui::Key::ArrowLeft),
                i.consume_key(none, egui::Key::Escape),
            )
        });
        if skip {
            return self.finish();
        }
        if back {
            self.back();
        }
        next && self.next()
    }

    /// Draws the dimmed backdrop, the spotlight and the current card. True
    /// when the tour just ended (finished or skipped).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        palette: Palette,
        spacing: Spacing,
        keybindings: &KeyBindings,
    ) -> bool {
        let Some(index) = self.step else {
            return false;
        };
        let step = &STEPS[index];
        let screen = ctx.screen_rect();
        let hole = self
            .anchors
            .rect(step.target)
            .map(|r| r.expand(2.0).intersect(screen))
            .filter(|r| r.is_positive());

        // Backdrop: swallows clicks and hovers so the UI behind stays still.
        egui::Area::new(egui::Id::new("tour_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                let painter = ui.painter();
                let dim = Color32::from_black_alpha(150);
                match hole {
                    Some(h) => {
                        for r in around(screen, h) {
                            painter.rect_filled(r, 0.0, dim);
                        }
                        painter.rect_stroke(
                            h,
                            4.0,
                            egui::Stroke::new(2.0_f32, palette.accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                    None => {
                        painter.rect_filled(screen, 0.0, dim);
                    }
                }
            });

        let pos = card_position(screen, hole, self.card_size);
        let mut ended = false;
        let card = egui::Area::new(egui::Id::new("tour_card"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                crate::ui::widgets::popup_frame(palette, spacing)
                    .stroke(egui::Stroke::new(1.0_f32, palette.border_strong))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(CARD_WIDTH - 32.0);
                        ui.label(
                            egui::RichText::new(format!("{} / {}", index + 1, STEPS.len()))
                                .small()
                                .color(palette.text_faint),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(step.title)
                                .size(17.0)
                                .strong()
                                .color(palette.text),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new((step.body)(keybindings)).color(palette.text_muted),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            let last = index + 1 == STEPS.len();
                            if !last
                                && ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Skip tour")
                                                .color(palette.text_faint),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                            {
                                ended = self.finish();
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let label = if last { "Get started" } else { "Next →" };
                                    let next = egui::Button::new(
                                        egui::RichText::new(label).color(palette.on_accent),
                                    )
                                    .fill(palette.accent);
                                    if ui.add(next).clicked() {
                                        ended = self.next();
                                    }
                                    if index > 0 && ui.button("← Back").clicked() {
                                        self.back();
                                    }
                                },
                            );
                        });
                    });
            });
        self.card_size = card.response.rect.size();
        ended
    }
}

/// The parts of `screen` outside `hole` (above, below, left, right).
fn around(screen: Rect, hole: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(screen.min, Pos2::new(screen.max.x, hole.min.y)),
        Rect::from_min_max(Pos2::new(screen.min.x, hole.max.y), screen.max),
        Rect::from_min_max(
            Pos2::new(screen.min.x, hole.min.y),
            Pos2::new(hole.min.x, hole.max.y),
        ),
        Rect::from_min_max(
            Pos2::new(hole.max.x, hole.min.y),
            Pos2::new(screen.max.x, hole.max.y),
        ),
    ]
}

/// Top-left corner of a `card`-sized card next to `hole`: on its right, left,
/// above or below, whichever fits first; inside it when it fills the window,
/// centred when there's no hole. Always kept within `screen`.
fn card_position(screen: Rect, hole: Option<Rect>, card: Vec2) -> Pos2 {
    let inner = screen.shrink(GAP);
    let clamp = |p: Pos2| {
        Pos2::new(
            p.x.clamp(inner.min.x, (inner.max.x - card.x).max(inner.min.x)),
            p.y.clamp(inner.min.y, (inner.max.y - card.y).max(inner.min.y)),
        )
    };
    let Some(h) = hole else {
        return clamp(screen.center() - card / 2.0);
    };
    let candidates = [
        // Right, left: vertically aligned with the top of the hole.
        Pos2::new(h.max.x + GAP, h.min.y + GAP),
        Pos2::new(h.min.x - GAP - card.x, h.min.y + GAP),
        // Above, below: horizontally centred on the hole.
        Pos2::new(h.center().x - card.x / 2.0, h.min.y - GAP - card.y),
        Pos2::new(h.center().x - card.x / 2.0, h.max.y + GAP),
    ];
    candidates
        .into_iter()
        .map(clamp)
        .find(|p| !Rect::from_min_size(*p, card).intersects(h))
        .unwrap_or_else(|| clamp(h.center() - card / 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))
    }
    const CARD: Vec2 = Vec2::new(380.0, 240.0);

    fn card_rect(hole: Option<Rect>) -> Rect {
        Rect::from_min_size(card_position(screen(), hole, CARD), CARD)
    }

    #[test]
    fn no_hole_centres_the_card() {
        assert_eq!(card_rect(None).center(), screen().center());
    }

    #[test]
    fn card_goes_right_of_a_left_panel() {
        let sidebar = Rect::from_min_max(Pos2::new(48.0, 24.0), Pos2::new(270.0, 680.0));
        let card = card_rect(Some(sidebar));
        assert!(card.min.x > sidebar.max.x);
        assert!(!card.intersects(sidebar));
    }

    #[test]
    fn card_goes_above_a_bottom_panel() {
        let terminal = Rect::from_min_max(Pos2::new(270.0, 680.0), Pos2::new(1400.0, 878.0));
        let card = card_rect(Some(terminal));
        assert!(card.max.y < terminal.min.y);
    }

    #[test]
    fn card_stays_on_screen_and_inside_a_huge_hole() {
        let editor = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(1390.0, 890.0));
        let card = card_rect(Some(editor));
        assert!(screen().contains_rect(card));
        assert_eq!(card.center(), editor.center());
    }

    #[test]
    fn backdrop_leaves_only_the_hole_uncovered() {
        let hole = Rect::from_min_max(Pos2::new(100.0, 200.0), Pos2::new(300.0, 400.0));
        let parts = around(screen(), hole);
        let area: f32 = parts.iter().map(|r| r.area()).sum();
        assert_eq!(area, screen().area() - hole.area());
        assert!(parts.iter().all(|r| !r.intersects(hole.shrink(0.5))));
    }

    #[test]
    fn keys_walk_the_steps_and_escape_ends() {
        let mut tour = Tour::new();
        assert!(tour.target().is_none());
        tour.start();
        assert_eq!(tour.target(), Some(Target::Center));
        assert!(!tour.next());
        assert_eq!(tour.target(), Some(Target::ActivityBar));
        tour.back();
        tour.back();
        assert_eq!(tour.step, Some(0));
        for _ in 1..STEPS.len() {
            assert!(!tour.next());
        }
        assert!(tour.next(), "next on the last step ends the tour");
        assert!(!tour.is_active());
    }

    #[test]
    fn every_step_has_text() {
        let kb = KeyBindings::default();
        for step in STEPS {
            assert!(!step.title.is_empty());
            assert!(!(step.body)(&kb).is_empty());
        }
    }
}
