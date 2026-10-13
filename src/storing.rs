//! storingUnicorns, the Unicorns' database client, opened from the activity
//! bar: started directly when it is installed, else its download page is
//! offered.

use std::path::{Path, PathBuf};

/// Download page of storingUnicorns.
pub const INSTALL_PAGE: &str = "https://ajustor.github.io/storingUnicorns/";

/// Command name of the storingUnicorns executable.
const COMMAND: &str = "storingUnicorns";

/// An installed storingUnicorns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// The executable (on the `PATH`, or where an installer put it).
    Executable(PathBuf),
    /// A macOS app bundle, started with `open`.
    MacApp(PathBuf),
}

/// Where storingUnicorns is installed, if it is: on the `PATH`, else in the
/// folders of its installers (the `PATH` the IDE started with may predate
/// the install).
pub fn locate() -> Option<Install> {
    if let Some(exe) = crate::dap::adapters::find_on_path(COMMAND) {
        return Some(Install::Executable(exe));
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    candidates(
        dirs_next::home_dir().as_deref(),
        env("LOCALAPPDATA").as_deref(),
        env("ProgramFiles").as_deref(),
    )
    .into_iter()
    .find(|c| match c {
        Install::Executable(p) => p.is_file(),
        Install::MacApp(p) => p.is_dir(),
    })
}

/// The places storingUnicorns' installers use, for this platform.
fn candidates(
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    program_files: Option<&Path>,
) -> Vec<Install> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let exe = format!("{COMMAND}.exe");
        // install.ps1 (per user), then the .msi.
        if let Some(d) = local_app_data {
            out.push(Install::Executable(
                d.join("Programs").join(COMMAND).join(&exe),
            ));
        }
        if let Some(d) = program_files {
            out.push(Install::Executable(d.join(COMMAND).join(&exe)));
        }
    } else if cfg!(target_os = "macos") {
        let app = format!("{COMMAND}.app");
        // install-macos.sh uses ~/Apps; the .dmg can go anywhere.
        if let Some(h) = home {
            out.push(Install::MacApp(h.join("Apps").join(&app)));
            out.push(Install::MacApp(h.join("Applications").join(&app)));
            out.push(Install::MacApp(h.join("Desktop").join(&app)));
        }
        out.push(Install::MacApp(Path::new("/Applications").join(&app)));
        if let Some(h) = home {
            out.push(Install::Executable(h.join(".local/bin").join(COMMAND)));
            out.push(Install::Executable(h.join(".cargo/bin").join(COMMAND)));
        }
    } else if let Some(h) = home {
        // install.sh, the AppImage's link, cargo install.
        out.push(Install::Executable(h.join(".local/bin").join(COMMAND)));
        out.push(Install::Executable(h.join(".cargo/bin").join(COMMAND)));
    }
    out
}

/// Start storingUnicorns, detached from the IDE.
pub fn launch(install: &Install) -> std::io::Result<()> {
    let mut cmd = match install {
        Install::Executable(exe) => std::process::Command::new(exe),
        Install::MacApp(app) => {
            let mut c = std::process::Command::new("open");
            c.arg("-a").arg(app);
            c
        }
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_folders_for_this_platform() {
        let home = Path::new("/home/u");
        let local = Path::new("/local");
        let pf = Path::new("/pf");
        let c = candidates(Some(home), Some(local), Some(pf));
        if cfg!(windows) {
            assert_eq!(
                c,
                [
                    Install::Executable(local.join("Programs/storingUnicorns/storingUnicorns.exe")),
                    Install::Executable(pf.join("storingUnicorns/storingUnicorns.exe")),
                ]
            );
        } else if cfg!(target_os = "macos") {
            assert_eq!(c[0], Install::MacApp(home.join("Apps/storingUnicorns.app")));
            assert!(c.contains(&Install::MacApp("/Applications/storingUnicorns.app".into())));
        } else {
            assert_eq!(
                c[0],
                Install::Executable(home.join(".local/bin/storingUnicorns"))
            );
        }
        assert!(
            cfg!(target_os = "macos") || candidates(None, None, None).is_empty(),
            "nothing without the folders"
        );
    }

    #[test]
    fn launching_a_missing_executable_fails() {
        let dir = tempfile::tempdir().unwrap();
        let missing = Install::Executable(dir.path().join("nope"));
        assert!(launch(&missing).is_err());
    }
}
