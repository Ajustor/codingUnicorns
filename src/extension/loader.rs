use std::path::Path;

/// Load a compiled extension dynamic library and return the plugin it exports.
///
/// # Safety
/// The `.so`/`.dll` must export `create_plugin` with the expected signature.
pub fn load_extension_lib(lib_path: &Path) -> anyhow::Result<Box<dyn crate::plugin::Plugin>> {
    use libloading::{Library, Symbol};
    unsafe {
        let lib = Library::new(lib_path)?;
        let constructor: Symbol<unsafe extern "C" fn() -> *mut dyn crate::plugin::Plugin> =
            lib.get(b"create_plugin")?;
        let raw = constructor();
        if raw.is_null() {
            anyhow::bail!("create_plugin returned null");
        }
        // We intentionally leak the library so the plugin remains valid.
        std::mem::forget(lib);
        Ok(Box::from_raw(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_is_an_error() {
        let p = std::env::temp_dir().join(format!("cu-missing-{}.dll", uuid::Uuid::new_v4()));
        assert!(load_extension_lib(&p).is_err());
    }

    #[test]
    fn non_library_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("fake.dll");
        std::fs::write(&p, b"definitely not a PE/ELF image").unwrap();
        assert!(load_extension_lib(&p).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn library_without_create_plugin_is_an_error() {
        // kernel32 is always loaded and does not export `create_plugin`.
        let err = load_extension_lib(Path::new("kernel32.dll"))
            .err()
            .expect("must fail");
        assert!(!err.to_string().is_empty());
    }
}
