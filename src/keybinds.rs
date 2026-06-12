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
