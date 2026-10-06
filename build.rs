fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Give the Windows executable its icon (Explorer, pinned taskbar, Alt+Tab before the
    // window icon is set). Checked against the target, not the host, so cross builds work.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            // A missing resource compiler shouldn't break the build; the app just keeps
            // the generic exe icon.
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
