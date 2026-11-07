//! `mount` / `unmount` (D-005). ISO files are attached directly; other
//! formats are converted to a temporary ISO in the data folder first and
//! that copy is deleted on unmount.

use crate::cli::Out;
use if_core::error::Exit;
use if_image::container::{self, Format};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "  mount IMAGE           Mount an image read only (non-ISO formats via a temporary ISO)\n\
  unmount IMAGE         Unmount it again\n";

/// Where the temporary ISO for `image` lives.
pub fn temp_iso(dirs: &if_core::paths::DataDirs, image: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(image).unwrap_or_else(|_| image.to_path_buf());
    let key = canon.display().to_string().to_lowercase();
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    let stem = image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    dirs.root.join("mounts").join(format!("{stem}-{h:016x}.iso"))
}

/// The ISO to attach for `image`, converting when needed.
pub fn iso_for(dirs: &if_core::paths::DataDirs, image: &Path) -> Result<(PathBuf, bool), (Exit, String)> {
    let (f, _) = container::detect(image).map_err(|e| (Exit::MediaIncompatible, e.to_string()))?;
    if f == Format::Iso {
        return Ok((image.to_path_buf(), false));
    }
    let t = temp_iso(dirs, image);
    if !t.is_file() {
        std::fs::create_dir_all(t.parent().expect("parent")).map_err(|e| (Exit::Io, e.to_string()))?;
        if_image::convert::convert(image, &t, if_image::convert::Output::Iso, &mut |_, _| true)
            .map_err(|e| (Exit::MediaIncompatible, e.to_string()))?;
    }
    Ok((t, true))
}

pub(crate) fn run(cmd: &str, tail: &[String], dirs: &if_core::paths::DataDirs, o: &mut Out<'_>) -> i32 {
    let path = match tail {
        [p] => PathBuf::from(p),
        [] => return o.fail(Exit::Usage, "missing IMAGE"),
        [_, x, ..] => return o.fail(Exit::Usage, &format!("unexpected argument: {x}")),
    };
    if !path.is_file() {
        return o.fail(Exit::Io, &format!("{} not found", path.display()));
    }
    match if cmd == "mount" { mount(&path, dirs) } else { unmount(&path, dirs) } {
        Ok(text) => {
            if o.json {
                o.ok_text(&json!({ "ok": true, "result": text }).to_string());
            } else {
                o.ok_text(&text);
            }
            0
        }
        Err((e, m)) => o.fail(e, &m),
    }
}

#[cfg(windows)]
fn mount(path: &Path, dirs: &if_core::paths::DataDirs) -> Result<String, (Exit, String)> {
    let (iso, temp) = iso_for(dirs, path)?;
    let at = crate::win::mount::mount(&iso).map_err(|m| (Exit::Generic, m))?;
    Ok(if temp {
        format!("Mounted at {at} (from a temporary ISO: {})", iso.display())
    } else {
        format!("Mounted at {at}")
    })
}

#[cfg(windows)]
fn unmount(path: &Path, dirs: &if_core::paths::DataDirs) -> Result<String, (Exit, String)> {
    let t = temp_iso(dirs, path);
    let (iso, temp) = if t.is_file() { (t, true) } else { (path.to_path_buf(), false) };
    crate::win::mount::unmount(&iso).map_err(|m| (Exit::Generic, m))?;
    if temp {
        let _ = std::fs::remove_file(&iso);
    }
    Ok("Unmounted".into())
}

#[cfg(not(windows))]
fn mount(_: &Path, _: &if_core::paths::DataDirs) -> Result<String, (Exit, String)> {
    Err((Exit::Generic, "mounting needs Windows".into()))
}

#[cfg(not(windows))]
fn unmount(_: &Path, _: &if_core::paths::DataDirs) -> Result<String, (Exit, String)> {
    Err((Exit::Generic, "mounting needs Windows".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_iso_is_stable_and_per_image() {
        let dirs = if_core::paths::DataDirs { root: PathBuf::from("/data"), portable: false };
        let a = temp_iso(&dirs, Path::new("/x/game.cue"));
        assert_eq!(a, temp_iso(&dirs, Path::new("/x/game.cue")));
        assert_ne!(a, temp_iso(&dirs, Path::new("/y/game.cue")));
        assert!(a.starts_with("/data/mounts") && a.extension().unwrap() == "iso");
    }
}
