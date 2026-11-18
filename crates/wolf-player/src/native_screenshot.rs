//! Native opcode1000 storage and platform JPEG encoder. Script expansion is
//! performed by the VM caller; no menu, map, or slot identity is inferred here.
use std::path::{Path, PathBuf};

pub(super) fn storage_path(root: &Path, save_root: &Path, path: &str) -> Result<PathBuf, String> {
    let path = wolf_assets::normalize_path(path).map_err(|e| e.to_string())?;
    if path
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("Save/"))
    {
        Ok(save_root.join(&path[5..]))
    } else {
        Ok(root.join(path))
    }
}

#[cfg(not(windows))]
pub(super) fn write_jpeg(_: &wolf_media::Image, _: &Path) -> Result<(), String> {
    Err("JPEG screenshot encoding is unavailable on this platform".into())
}

#[cfg(windows)]
pub(super) fn write_jpeg(image: &wolf_media::Image, path: &Path) -> Result<(), String> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    use windows::{
        core::{GUID, PCWSTR},
        Win32::Graphics::GdiPlus::*,
    };
    fn checked(status: Status, operation: &str) -> Result<(), String> {
        if status == Ok {
            Result::Ok(())
        } else {
            Err(format!("JPEG {operation}: GDI+ status {}", status.0))
        }
    }
    struct Session(usize);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe { GdiplusShutdown(self.0) };
        }
    }
    struct Bitmap(*mut GpBitmap);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe {
                GdipDisposeImage(self.0.cast());
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let (width, height) = image.dimensions();
    let mut pixels = image.bytes().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut token = 0;
    unsafe {
        checked(
            GdiplusStartup(
                &mut token,
                &GdiplusStartupInput {
                    GdiplusVersion: 1,
                    ..Default::default()
                },
                ptr::null_mut(),
            ),
            "startup",
        )?;
    }
    let _session = Session(token);
    let mut bitmap = ptr::null_mut();
    unsafe {
        checked(
            GdipCreateBitmapFromScan0(
                width as i32,
                height as i32,
                (width * 4) as i32,
                0x26200a,
                Some(pixels.as_ptr()),
                &mut bitmap,
            ),
            "bitmap",
        )?;
    }
    let bitmap = Bitmap(bitmap);
    // JPEG CLSID and EncoderQuality are platform identifiers, independent of
    // the script-selected filename. The original passes quality85 at A2DBC1.
    let codec = GUID::from_u128(0x557cf401_1a04_11d3_9a73_0000f81ef32e);
    let quality = 85_u32;
    let parameters = EncoderParameters {
        Count: 1,
        Parameter: [EncoderParameter {
            Guid: EncoderQuality,
            NumberOfValues: 1,
            Type: EncoderParameterValueTypeLong.0 as u32,
            Value: (&quality as *const u32).cast::<c_void>().cast_mut(),
        }],
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        checked(
            GdipSaveImageToFile(bitmap.0.cast(), PCWSTR(wide.as_ptr()), &codec, &parameters),
            "write",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_paths_use_the_isolated_save_store() {
        assert_eq!(
            storage_path(Path::new("game"), Path::new("isolated"), "Save/ss7.jpg").unwrap(),
            Path::new("isolated/ss7.jpg")
        );
        assert!(storage_path(Path::new("game"), Path::new("isolated"), "../outside.jpg").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn jpeg_screenshot_roundtrips_dimensions_and_channels() {
        let path = std::env::temp_dir().join(format!("wolf-screenshot-{}.jpg", std::process::id()));
        let image = wolf_media::Image::new(16, 8, [240, 20, 40, 255].repeat(128)).unwrap();
        write_jpeg(&image, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8]));
        let decoded = wolf_media::Image::from_encoded(&bytes).unwrap();
        assert_eq!(decoded.dimensions(), (16, 8));
        assert!(decoded.bytes()[0] > 220 && decoded.bytes()[1] < 40 && decoded.bytes()[2] < 60);
        let replacement = wolf_media::Image::new(8, 16, [20, 230, 40, 255].repeat(128)).unwrap();
        write_jpeg(&replacement, &path).unwrap();
        let decoded = wolf_media::Image::from_encoded(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded.dimensions(), (8, 16));
        assert!(decoded.bytes()[0] < 40 && decoded.bytes()[1] > 210);
        std::fs::remove_file(path).unwrap();
    }
}
