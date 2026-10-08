//! Spawning helpers shared by every background process.

use std::process::Command;

/// `CREATE_NO_WINDOW` process creation flag (Windows).
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub trait CommandExt {
    /// Run a console program without a console window.
    ///
    /// On Windows the IDE has no console (`windows_subsystem = "windows"`), so
    /// every console program it starts — language servers, debug adapters,
    /// `cargo`, `npm`, `git`… — would open a window of its own, and so would
    /// each process *those* start (csharp-ls runs `dotnet` and MSBuild). With
    /// this flag the child gets a hidden console that its own children share.
    /// No-op elsewhere.
    fn no_window(&mut self) -> &mut Self;
}

impl CommandExt for Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CommandExt;

    /// The flag must not stop the child from running or from talking over pipes.
    #[test]
    fn hidden_child_still_runs_and_pipes_output() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "echo hello"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "echo hello"]);
            c
        };
        let out = cmd.no_window().output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }
}
