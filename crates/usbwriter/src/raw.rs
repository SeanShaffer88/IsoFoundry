//! Raw streaming copies between an image file and a device (the
//! `usb-image-to-usb` / `usb-usb-to-image` variants, dd-style).

use crate::device::BlockDevice;
use crate::{err, Result};
use if_core::job::{CancelToken, Progress};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const CHUNK: usize = 4 << 20;

/// Copies an image file onto a device from offset 0. The image must
/// fit. Progress reports bytes; cancelling stops between chunks (the
/// device is left partially written — raw mode has no undo).
pub fn image_to_device(
    image: &Path,
    dev: &mut dyn BlockDevice,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<u64> {
    let mut f = File::open(image)?;
    let total = f.metadata()?.len();
    if total > dev.size() {
        return err(format!("the image ({total} bytes) is larger than the target ({} bytes)", dev.size()));
    }
    progress.set_total(total);
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        if cancel.is_cancelled() {
            return err("cancelled");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dev.write_at(done, &buf[..n])?;
        done += n as u64;
        progress.add(n as u64);
    }
    dev.flush()?;
    Ok(done)
}

/// Copies a device (or the first `limit` bytes of it) into an image
/// file. The output goes to a `.part` file first and is renamed when
/// complete, matching the house atomic-write rule.
pub fn device_to_image(
    dev: &dyn BlockDevice,
    out: &Path,
    limit: Option<u64>,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<u64> {
    let total = limit.unwrap_or_else(|| dev.size());
    if total > dev.size() {
        return err("read limit is beyond the end of the device");
    }
    let part_path = {
        let mut s = out.as_os_str().to_owned();
        s.push(".part");
        std::path::PathBuf::from(s)
    };
    let mut f = File::create(&part_path)?;
    progress.set_total(total);
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < total {
        if cancel.is_cancelled() {
            drop(f);
            let _ = std::fs::remove_file(&part_path);
            return err("cancelled");
        }
        let want = ((total - done) as usize).min(CHUNK);
        dev.read_at(done, &mut buf[..want])?;
        f.write_all(&buf[..want])?;
        done += want as u64;
        progress.add(want as u64);
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(&part_path, out)?;
    Ok(done)
}

/// SHA-256 of a byte range on a device, for post-write verification.
pub fn hash_device_range(dev: &dyn BlockDevice, len: u64) -> Result<String> {
    hash_range(&mut |off, buf| dev.read_at(off, buf), len)
}

/// SHA-256 of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    hash_range(
        &mut |off, buf| {
            f.seek(SeekFrom::Start(off))?;
            f.read_exact(buf)?;
            Ok(())
        },
        len,
    )
}

fn hash_range(read: &mut dyn FnMut(u64, &mut [u8]) -> Result<()>, len: u64) -> Result<String> {
    let mut h = if_hash::Hasher::new(if_hash::Algorithms { crc32: false, md5: false, sha1: false, sha256: true });
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < len {
        let want = ((len - done) as usize).min(CHUNK);
        read(done, &mut buf[..want])?;
        h.update(&buf[..want]);
        done += want as u64;
    }
    h.finish().sha256.ok_or_else(|| crate::Error("hash failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::FileDevice;

    #[test]
    fn raw_roundtrip_and_verify() {
        let dir = std::env::temp_dir().join(format!("usbw-raw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("a.img");
        let data: Vec<u8> = (0..5_000_000u32).map(|i| (i * 7 % 253) as u8).collect();
        std::fs::write(&img, &data).unwrap();
        let dev_path = dir.join("dev.img");
        let mut dev = FileDevice::create(&dev_path, 8 << 20).unwrap();
        let progress = Progress::default();
        let cancel = CancelToken::new();
        let n = image_to_device(&img, &mut dev, &progress, &cancel).unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(hash_device_range(&dev, n).unwrap(), hash_file(&img).unwrap());
        let out = dir.join("b.img");
        let m = device_to_image(&dev, &out, Some(n), &progress, &cancel).unwrap();
        assert_eq!(m, n);
        assert_eq!(std::fs::read(&out).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_too_large_refused() {
        let dir = std::env::temp_dir().join(format!("usbw-raw2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("big.img");
        std::fs::write(&img, vec![0u8; 4096]).unwrap();
        let mut dev = FileDevice::create(&dir.join("dev.img"), 1024).unwrap();
        let r = image_to_device(&img, &mut dev, &Progress::default(), &CancelToken::new());
        assert!(r.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
