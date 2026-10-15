//! How the IDE reaches a language server.

/// A language server the IDE starts, or one another program already runs
/// (the Godot editor runs its own on a TCP port).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspTarget {
    /// Start `command` with `args` and talk to it over stdio.
    Spawn { command: String, args: Vec<String> },
    /// Connect to a server already listening on `host:port`.
    Connect(String),
}

impl LspTarget {
    /// The address of a server the IDE connects to; `None` for one it starts.
    pub fn connect_addr(&self) -> Option<&str> {
        match self {
            Self::Connect(addr) => Some(addr),
            Self::Spawn { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_connected_servers_have_an_address() {
        assert_eq!(
            LspTarget::Connect("127.0.0.1:6005".into()).connect_addr(),
            Some("127.0.0.1:6005")
        );
        let spawn = LspTarget::Spawn {
            command: "pylsp".into(),
            args: vec![],
        };
        assert_eq!(spawn.connect_addr(), None);
    }
}
