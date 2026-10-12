//! Files that modules download on first use (debug adapters, language
//! servers): fetched over HTTPS into the module's folder. A `.zip` or
//! `.tar.gz` is unpacked; a lone `.gz` or a bare executable becomes the
//! module's `binary`.

use std::path::Path;

/// Largest archive accepted.
const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;

/// How a download is unpacked, from its URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    TarGz,
    /// A single gzip-compressed file (`rust-analyzer-….gz`).
    Gz,
    /// The executable itself (`marksman-linux-x64`, `marksman.exe`).
    Raw,
}

impl Format {
    pub fn of(url: &str) -> Self {
        let name = url.rsplit('/').next().unwrap_or(url).to_ascii_lowercase();
        if name.ends_with(".zip") {
            Self::Zip
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Self::TarGz
        } else if name.ends_with(".gz") {
            Self::Gz
        } else {
            Self::Raw
        }
    }
}

/// Download `url` into `dest` (blocking). An archive is unpacked there; a
/// single file is saved as `binary` (relative to `dest`, `/`-separated, with
/// the executable suffix added) and made executable.
pub fn download(url: &str, dest: &Path, binary: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("refusing non-https download {url}"));
    }
    let bytes = crate::extension::remote_registry::http_fetch(url, MAX_ARCHIVE_BYTES)?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let unpacked = |result: Result<(), String>| {
        result?;
        // Archives made on Windows don't keep the executable bit.
        let target = dest.join(executable_name(binary));
        let target = resolve_glob(&target.to_string_lossy());
        make_executable(Path::new(&target))
    };
    match Format::of(url) {
        Format::Zip => unpacked(extract_zip(&bytes, dest)),
        Format::TarGz => unpacked(extract_tar_gz(&bytes, dest)),
        Format::Gz => {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut out)
                .map_err(|e| format!("cannot decompress {url}: {e}"))?;
            write_executable(&dest.join(executable_name(binary)), &out)
        }
        Format::Raw => write_executable(&dest.join(executable_name(binary)), &bytes),
    }
}

/// `dir/name_*.jar` → the matching file (the last one in name order, i.e. the
/// newest version when several are there); unchanged when nothing matches or
/// the `*` is not in the file name.
pub fn resolve_glob(arg: &str) -> String {
    let Some(sep) = arg.rfind(['/', '\\']) else {
        return arg.to_string();
    };
    let (dir, pattern) = (&arg[..sep], &arg[sep + 1..]);
    if dir.contains('*') || pattern.matches('*').count() != 1 {
        return arg.to_string();
    }
    let (prefix, suffix) = pattern.split_once('*').expect("one star");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return arg.to_string();
    };
    let found = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.len() >= prefix.len() + suffix.len())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .max();
    match found {
        Some(name) => format!("{dir}{}{name}", &arg[sep..sep + 1]),
        None => arg.to_string(),
    }
}

/// `binary` with the platform's executable suffix, unless it has one.
fn executable_name(binary: &str) -> String {
    let suffix = std::env::consts::EXE_SUFFIX;
    if suffix.is_empty() || binary.to_ascii_lowercase().ends_with(suffix) {
        binary.to_string()
    } else {
        format!("{binary}{suffix}")
    }
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    make_executable(path)
}

/// Mark `path` executable (Unix), when it exists.
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if path.is_file() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // `enclosed_name` rejects absolute paths and `..` components.
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// `.tar.gz` archives are unpacked with the system `tar`.
pub(crate) fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use crate::process_ext::CommandExt as _;
    let archive = dest.join("download.tar.gz");
    std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .no_window()
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(dest)
        .status();
    let _ = std::fs::remove_file(&archive);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("tar failed ({s})")),
        Err(e) => Err(format!("cannot run tar: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_follows_the_file_name() {
        assert_eq!(Format::of("https://x/a.zip"), Format::Zip);
        assert_eq!(Format::of("https://x/a.tar.gz"), Format::TarGz);
        assert_eq!(Format::of("https://x/a.TGZ"), Format::TarGz);
        assert_eq!(Format::of("https://x/rust-analyzer-x86_64.gz"), Format::Gz);
        assert_eq!(Format::of("https://x/marksman.exe"), Format::Raw);
        assert_eq!(Format::of("https://x/marksman-linux-x64"), Format::Raw);
        assert_eq!(Format::of("https://x/v1.0/marksman-macos"), Format::Raw);
    }

    #[test]
    fn executable_name_adds_the_suffix_once() {
        let exe = std::env::consts::EXE_SUFFIX;
        assert_eq!(executable_name("bin/srv"), format!("bin/srv{exe}"));
        if !exe.is_empty() {
            assert_eq!(executable_name("srv.EXE"), "srv.EXE");
        }
    }

    #[test]
    fn single_files_are_written_executable() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(executable_name("bin/srv"));
        write_executable(&target, b"#!/bin/sh").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"#!/bin/sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn insecure_urls_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = download("http://example.invalid/a.zip", dir.path(), "a").unwrap_err();
        assert!(err.contains("non-https"));
    }
}
