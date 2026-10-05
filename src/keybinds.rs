//! Central registry for application-command keyboard shortcuts. Built-ins and
//! panels/modules register here so clashes are detected instead of silently
//! shadowing each other. (Editor text-editing keys are handled separately.)

/// A key chord: modifiers + a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: egui::Key,
}

impl Chord {
    pub fn new(ctrl: bool, shift: bool, alt: bool, key: egui::Key) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            key,
        }
    }
    /// Common helpers.
    pub fn ctrl(key: egui::Key) -> Self {
        Self::new(true, false, false, key)
    }
    pub fn ctrl_shift(key: egui::Key) -> Self {
        Self::new(true, true, false, key)
    }

    /// True if this exact chord was pressed this frame.
    pub fn matches(&self, i: &egui::InputState) -> bool {
        i.modifiers.ctrl == self.ctrl
            && i.modifiers.shift == self.shift
            && i.modifiers.alt == self.alt
            && i.key_pressed(self.key)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        let key = format!("{:?}", self.key);
        write!(
            f,
            "{}",
            parts
                .into_iter()
                .chain(std::iter::once(key.as_str()))
                .collect::<Vec<_>>()
                .join("+")
        )
    }
}

/// One registered command shortcut.
#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub chord: Chord,
    pub description: String,
}

#[derive(Default)]
pub struct KeybindingRegistry {
    bindings: Vec<Binding>,
}

impl KeybindingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command shortcut. Returns the id of an existing binding that
    /// already uses this exact chord (a conflict), if any. The new binding is
    /// still added, but the caller can warn about the clash.
    pub fn register(&mut self, id: &str, chord: Chord, description: &str) -> Option<String> {
        let conflict = self
            .bindings
            .iter()
            .find(|b| b.chord == chord)
            .map(|b| b.id.clone());
        self.bindings.push(Binding {
            id: id.to_string(),
            chord,
            description: description.to_string(),
        });
        conflict
    }

    /// Return the id of the command whose chord was pressed this frame, if any.
    pub fn triggered(&self, i: &egui::InputState) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.chord.matches(i))
            .map(|b| b.id.as_str())
    }

    /// All chords mapped to more than one command id (for diagnostics / settings UI).
    pub fn conflicts(&self) -> Vec<(Chord, Vec<String>)> {
        let mut out: Vec<(Chord, Vec<String>)> = Vec::new();
        for b in &self.bindings {
            if let Some(entry) = out.iter_mut().find(|(c, _)| *c == b.chord) {
                entry.1.push(b.id.clone());
            } else {
                out.push((b.chord, vec![b.id.clone()]));
            }
        }
        out.retain(|(_, ids)| ids.len() > 1);
        out
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};

    /// Run `f` against the `InputState` of a headless frame in which `key` was
    /// pressed while `mods` were held (or no key at all when `key` is None).
    fn with_input<R>(key: Option<Key>, mods: Modifiers, f: impl Fn(&egui::InputState) -> R) -> R {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput {
            modifiers: mods,
            ..Default::default()
        };
        if let Some(key) = key {
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            });
        }
        let mut out = None;
        let _ = ctx.run(raw, |ctx| out = Some(ctx.input(&f)));
        out.unwrap()
    }

    #[test]
    fn helper_constructors_set_expected_modifiers() {
        assert_eq!(Chord::ctrl(Key::S), Chord::new(true, false, false, Key::S));
        assert_eq!(
            Chord::ctrl_shift(Key::P),
            Chord::new(true, true, false, Key::P)
        );
    }

    #[test]
    fn display_lists_modifiers_in_fixed_order_then_key() {
        assert_eq!(Chord::new(false, false, false, Key::F5).to_string(), "F5");
        assert_eq!(Chord::ctrl(Key::S).to_string(), "Ctrl+S");
        assert_eq!(
            Chord::new(true, true, true, Key::ArrowUp).to_string(),
            "Ctrl+Shift+Alt+ArrowUp"
        );
        assert_eq!(
            Chord::new(false, true, true, Key::Tab).to_string(),
            "Shift+Alt+Tab"
        );
    }

    #[test]
    fn matches_requires_exact_modifiers_and_key() {
        let chord = Chord::ctrl(Key::S);
        assert!(with_input(Some(Key::S), Modifiers::CTRL, |i| chord.matches(i)));
        // Extra modifier -> no match.
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert!(!with_input(Some(Key::S), ctrl_shift, |i| chord.matches(i)));
        // Missing modifier -> no match.
        assert!(!with_input(Some(Key::S), Modifiers::NONE, |i| chord.matches(i)));
        // Wrong key -> no match.
        assert!(!with_input(Some(Key::D), Modifiers::CTRL, |i| chord.matches(i)));
        // Modifiers held but no key press.
        assert!(!with_input(None, Modifiers::CTRL, |i| chord.matches(i)));
        // Alt chords.
        let alt = Chord::new(false, false, true, Key::ArrowLeft);
        assert!(with_input(Some(Key::ArrowLeft), Modifiers::ALT, |i| alt.matches(i)));
    }

    #[test]
    fn register_reports_conflicts_but_keeps_both_bindings() {
        let mut reg = KeybindingRegistry::new();
        assert!(reg.bindings().is_empty());
        assert_eq!(reg.register("save", Chord::ctrl(Key::S), "Save"), None);
        assert_eq!(reg.register("find", Chord::ctrl(Key::F), "Find"), None);
        assert_eq!(
            reg.register("save_all", Chord::ctrl(Key::S), "Save all"),
            Some("save".to_string())
        );
        let b = reg.bindings();
        assert_eq!(b.len(), 3);
        assert_eq!(b[2].id, "save_all");
        assert_eq!(b[2].description, "Save all");
        assert_eq!(b[2].chord, Chord::ctrl(Key::S));
    }

    #[test]
    fn triggered_returns_first_registered_matching_command() {
        let mut reg = KeybindingRegistry::new();
        reg.register("save", Chord::ctrl(Key::S), "");
        reg.register("palette", Chord::ctrl_shift(Key::P), "");
        reg.register("save_dup", Chord::ctrl(Key::S), "");

        assert_eq!(
            with_input(Some(Key::S), Modifiers::CTRL, |i| reg
                .triggered(i)
                .map(str::to_string)),
            Some("save".to_string())
        );
        assert_eq!(
            with_input(Some(Key::P), Modifiers::CTRL | Modifiers::SHIFT, |i| reg
                .triggered(i)
                .map(str::to_string)),
            Some("palette".to_string())
        );
        assert_eq!(
            with_input(Some(Key::P), Modifiers::CTRL, |i| reg
                .triggered(i)
                .map(str::to_string)),
            None
        );
    }

    #[test]
    fn conflicts_groups_ids_per_chord_and_omits_unique_chords() {
        let mut reg = KeybindingRegistry::new();
        assert!(reg.conflicts().is_empty());
        reg.register("a", Chord::ctrl(Key::A), "");
        reg.register("b", Chord::ctrl(Key::B), "");
        reg.register("a2", Chord::ctrl(Key::A), "");
        reg.register("a3", Chord::ctrl(Key::A), "");
        reg.register("b2", Chord::ctrl(Key::B), "");
        reg.register("c", Chord::ctrl(Key::C), "");
        let c = reg.conflicts();
        assert_eq!(
            c,
            vec![
                (
                    Chord::ctrl(Key::A),
                    vec!["a".to_string(), "a2".to_string(), "a3".to_string()]
                ),
                (Chord::ctrl(Key::B), vec!["b".to_string(), "b2".to_string()]),
            ]
        );
    }
}
