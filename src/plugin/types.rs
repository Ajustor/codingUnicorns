/// A command that can be registered by a plugin and shown in the command palette.
#[derive(Clone)]
pub struct PluginCommand {
    pub id: String,
    pub title: String,
    pub keybinding: Option<String>,
}

/// A sidebar panel contributed by a plugin.
#[derive(Clone)]
pub struct SidebarPanel {
    pub id: String,
    pub title: String,
    pub icon: &'static str,
}

/// Context passed to plugins each frame.
pub struct PluginContext<'a> {
    pub buffer_text: &'a str,
    pub filename: Option<&'a str>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub is_modified: bool,
    /// The symbol currently being hovered (if any), for hover-doc queries.
    pub hovered_word: Option<&'a str>,
}

/// What a plugin can tell the IDE to do.
#[derive(Default)]
pub struct PluginResponse {
    pub status_text: Option<String>,
    pub notifications: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_default_is_empty() {
        let r = PluginResponse::default();
        assert!(r.status_text.is_none());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn command_and_panel_clone() {
        let c = PluginCommand {
            id: "a.b".into(),
            title: "A: B".into(),
            keybinding: Some("Ctrl+B".into()),
        };
        let c2 = c.clone();
        assert_eq!(c2.id, "a.b");
        assert_eq!(c2.keybinding.as_deref(), Some("Ctrl+B"));
        let p = SidebarPanel {
            id: "p".into(),
            title: "Panel".into(),
            icon: "*",
        };
        assert_eq!(p.clone().icon, "*");
    }

    #[test]
    fn context_holds_borrowed_state() {
        let text = String::from("abc");
        let ctx = PluginContext {
            buffer_text: &text,
            filename: None,
            cursor_row: 1,
            cursor_col: 2,
            is_modified: true,
            hovered_word: Some("abc"),
        };
        assert_eq!(ctx.buffer_text, "abc");
        assert!(ctx.is_modified);
        assert_eq!((ctx.cursor_row, ctx.cursor_col), (1, 2));
        assert_eq!(ctx.hovered_word, Some("abc"));
        assert!(ctx.filename.is_none());
    }
}
