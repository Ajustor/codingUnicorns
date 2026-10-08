pub mod account;
pub mod panel;
pub mod permission;
pub mod process;
pub mod protocol;
pub mod session;

/// Hide the console window of a CLI started from the IDE, which has no console
/// on Windows: without this flag every `claude` run would flash one.
pub(crate) fn no_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
