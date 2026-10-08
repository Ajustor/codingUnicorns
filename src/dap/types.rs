use std::path::PathBuf;

/// How to start a debug adapter and what to launch with it.
#[derive(Debug, Clone, Default)]
pub struct DapConfig {
    /// The debug adapter binary (e.g. "codelldb", "python3", "node").
    pub adapter_cmd: String,
    /// Arguments for the adapter (e.g. ["-m", "debugpy.adapter"]).
    pub adapter_args: Vec<String>,
    /// The `launch` request body sent after `configurationDone`.
    /// Use `${file}` and `${workspaceFolder}` as placeholders.
    pub launch_config: serde_json::Value,
    /// stdio, or TCP (`${port}` in `adapter_args`).
    pub transport: crate::extension::manifest::DebuggerTransport,
}

/// A source breakpoint (before or after DAP verification).
#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// Set to true once the DAP server acknowledges it.
    pub verified: bool,
    pub id: Option<i64>,
}

/// One frame in the call stack.
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub file: Option<PathBuf>,
    /// 1-based line number.
    pub line: usize,
}

/// A debug variable (or scope entry).
#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub var_type: Option<String>,
    /// Non-zero when this variable can be expanded (has children).
    pub variables_reference: i64,
}

/// A variable scope of a stack frame (Locals, Globals, Registers…).
#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub name: String,
    /// Reference to pass to a `variables` request (0 = no variables).
    pub variables_reference: i64,
    /// The adapter flagged this scope as costly to fetch: it is shown
    /// collapsed and its variables are only requested on expand.
    pub expensive: bool,
}

/// Latest evaluation of a watch expression.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchResult {
    /// Evaluation requested, no answer yet (or the session is not paused).
    Pending,
    Value {
        value: String,
        var_type: Option<String>,
        /// Non-zero when the result is structured and can be expanded.
        variables_reference: i64,
    },
    Error(String),
}

/// Lifecycle state of the active debug session.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum DebugSessionState {
    #[default]
    Idle,
    /// Downloading the adapter or running the `preLaunchTask`.
    Preparing,
    Launching,
    Running,
    Paused {
        thread_id: i64,
    },
    Terminated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_state_defaults_to_idle() {
        assert_eq!(DebugSessionState::default(), DebugSessionState::Idle);
    }

    #[test]
    fn paused_states_compare_by_thread() {
        assert_eq!(
            DebugSessionState::Paused { thread_id: 1 },
            DebugSessionState::Paused { thread_id: 1 }
        );
        assert_ne!(
            DebugSessionState::Paused { thread_id: 1 },
            DebugSessionState::Paused { thread_id: 2 }
        );
        assert_ne!(DebugSessionState::Running, DebugSessionState::Launching);
    }

    #[test]
    fn types_are_cloneable_and_debuggable() {
        let cfg = DapConfig {
            transport: Default::default(),
            adapter_cmd: "python3".into(),
            adapter_args: vec!["-m".into(), "debugpy.adapter".into()],
            launch_config: serde_json::json!({"program": "${file}"}),
        };
        let c = cfg.clone();
        assert_eq!(c.adapter_args, cfg.adapter_args);
        assert!(format!("{c:?}").contains("debugpy.adapter"));

        let bp = Breakpoint {
            file: PathBuf::from("a.rs"),
            line: 3,
            verified: true,
            id: Some(7),
        };
        assert_eq!(bp.clone().id, Some(7));
        assert!(format!("{bp:?}").contains("a.rs"));

        let f = StackFrame {
            id: 1,
            name: "main".into(),
            file: None,
            line: 1,
        };
        assert_eq!(f.clone().name, "main");
        assert!(format!("{f:?}").contains("main"));

        let v = Variable {
            name: "x".into(),
            value: "1".into(),
            var_type: Some("i32".into()),
            variables_reference: 0,
        };
        assert_eq!(v.clone().var_type.as_deref(), Some("i32"));
        assert!(format!("{v:?}").contains("i32"));
        assert!(format!("{:?}", DebugSessionState::Terminated).contains("Terminated"));
    }
}
