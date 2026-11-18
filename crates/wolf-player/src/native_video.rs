//! Windows software video decoding. PTS controls presentation; the VM owns no COM objects.
//! This backend does not claim that MF reproduces the original decoder's RGB rounding.
use std::{path::Path, sync::Arc};
use wolf_media::Image;

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{marker::PhantomData, os::windows::ffi::OsStrExt, rc::Rc};
    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            Media::MediaFoundation::*,
            System::{
                Com::{
                    CoInitializeEx, CoUninitialize,
                    StructuredStorage::{PropVariantClear, PROPVARIANT},
                    COINIT_MULTITHREADED,
                },
                Variant::{VT_I8, VT_UI8},
            },
        },
    };

    struct Runtime {
        owns_com: bool,
        _thread: PhantomData<Rc<()>>,
    }
    impl Runtime {
        fn new() -> Result<Self, String> {
            unsafe {
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                let owns_com = hr.is_ok();
                if !owns_com && hr != RPC_E_CHANGED_MODE {
                    return Err(format!("CoInitializeEx: {hr:?}"));
                }
                if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                    if owns_com {
                        CoUninitialize();
                    }
                    return Err(format!("MFStartup: {e}"));
                }
                Ok(Self {
                    owns_com,
                    _thread: PhantomData,
                })
            }
        }
    }
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                if self.owns_com {
                    CoUninitialize();
                }
            }
        }
    }

    pub(super) struct Decoder {
        // Fields drop in declaration order: reader releases before MF/COM shutdown.
        reader: IMFSourceReader,
        _runtime: Runtime,
        width: u32,
        height: u32,
        stride: i32,
        duration: i64,
        pub(super) ended: bool,
    }
    impl Decoder {
        pub fn open(path: &Path) -> Result<Self, String> {
            let runtime = Runtime::new()?;
            let absolute = path
                .canonicalize()
                .map_err(|e| format!("video path: {e}"))?;
            // MF's URL resolver rejects Rust's verbatim \\?\ drive prefix.
            let ordinary = absolute.to_string_lossy();
            let ordinary = ordinary.strip_prefix(r"\\?\").unwrap_or(&ordinary);
            let wide: Vec<u16> = std::ffi::OsStr::new(ordinary)
                .encode_wide()
                .chain(Some(0))
                .collect();
            unsafe {
                let mut attrs = None;
                MFCreateAttributes(&mut attrs, 2).map_err(err)?;
                let attrs = attrs.ok_or("MFCreateAttributes returned no attributes")?;
                attrs
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                    .map_err(err)?;
                // No D3D manager: SourceReader returns CPU buffers, independent of renderer.
                let reader =
                    MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs).map_err(err)?;
                reader
                    .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                    .map_err(err)?;
                reader.SetStreamSelection(stream(), true).map_err(err)?;
                let media = MFCreateMediaType().map_err(err)?;
                media
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .map_err(err)?;
                media
                    .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                    .map_err(err)?;
                reader
                    .SetCurrentMediaType(stream(), None, &media)
                    .map_err(err)?;
                let output = reader.GetCurrentMediaType(stream()).map_err(err)?;
                let size = output.GetUINT64(&MF_MT_FRAME_SIZE).map_err(err)?;
                let width = (size >> 32) as u32;
                let height = size as u32;
                if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_000_000 {
                    return Err("video dimensions exceed 16 megapixel limit".into());
                }
                let stride = output
                    .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                    .map(|v| v as i32)
                    .unwrap_or((width * 4) as i32);
                if stride.unsigned_abs() < width * 4
                    || u64::from(stride.unsigned_abs()) * u64::from(height) > 64 << 20
                {
                    return Err("video output stride exceeds bounds".into());
                }
                let mut value = reader
                    .GetPresentationAttribute(
                        MF_SOURCE_READER_MEDIASOURCE.0 as u32,
                        &MF_PD_DURATION,
                    )
                    .map_err(err)?;
                let value_inner = &value.Anonymous.Anonymous;
                let duration = if value_inner.vt == VT_UI8 {
                    i64::try_from(value_inner.Anonymous.uhVal).unwrap_or(0)
                } else {
                    0
                };
                PropVariantClear(&mut value).map_err(err)?;
                if duration <= 0 {
                    return Err("video lacks a positive 100 ns duration".into());
                }
                Ok(Self {
                    reader,
                    _runtime: runtime,
                    width,
                    height,
                    stride,
                    duration,
                    ended: false,
                })
            }
        }
        pub fn dimensions(&self) -> (u32, u32) {
            (self.width, self.height)
        }
        pub fn duration_100ns(&self) -> i64 {
            self.duration
        }
        pub(super) fn seek_zero(&mut self) -> Result<(), String> {
            unsafe {
                let mut value = PROPVARIANT::default();
                (*value.Anonymous.Anonymous).vt = VT_I8;
                (*value.Anonymous.Anonymous).Anonymous.hVal = 0;
                self.reader
                    .SetCurrentPosition(&GUID::zeroed(), &value)
                    .map_err(err)?;
            }
            self.ended = false;
            Ok(())
        }

        pub(super) fn read_frame(&mut self) -> Result<Option<(i64, Arc<Image>)>, String> {
            unsafe {
                let mut flags = 0;
                let mut timestamp = 0;
                let mut sample = None;
                self.reader
                    .ReadSample(
                        stream(),
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                    .map_err(err)?;
                if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                    return Err("dynamic video format change is unsupported".into());
                }
                self.ended = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
                let Some(sample) = sample else {
                    return Ok(None);
                };
                let buffer = sample.ConvertToContiguousBuffer().map_err(err)?;
                let mut pointer = std::ptr::null_mut();
                let mut length = 0;
                buffer
                    .Lock(&mut pointer, None, Some(&mut length))
                    .map_err(err)?;
                // Always unlock, including invalid layout errors, before releasing buffer.
                let result = (|| {
                    let stride = self.stride.unsigned_abs() as usize;
                    let required = stride
                        .checked_mul(self.height as usize)
                        .ok_or("video stride overflow")?;
                    if pointer.is_null() || required > length as usize {
                        return Err("short RGB32 video buffer".to_string());
                    }
                    let source = std::slice::from_raw_parts(pointer, required);
                    let mut rgba = vec![0; self.width as usize * self.height as usize * 4];
                    for y in 0..self.height as usize {
                        let source_y = if self.stride < 0 {
                            self.height as usize - y - 1
                        } else {
                            y
                        };
                        for x in 0..self.width as usize {
                            let s = source_y * stride + x * 4;
                            let d = (y * self.width as usize + x) * 4;
                            rgba[d..d + 4].copy_from_slice(&[
                                source[s + 2],
                                source[s + 1],
                                source[s],
                                255,
                            ]);
                        }
                    }
                    Ok((
                        timestamp,
                        Arc::new(Image::new(self.width as usize, self.height as usize, rgba)?),
                    ))
                })();
                let unlock = buffer.Unlock().map_err(err);
                let frame = result?;
                unlock?;
                Ok(Some(frame))
            }
        }
    }
    fn stream() -> u32 {
        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32
    }
    fn err(e: windows::core::Error) -> String {
        format!("Media Foundation: {e}")
    }
}

#[cfg(windows)]
#[path = "native_video/worker.rs"]
mod worker;
#[cfg(windows)]
pub use worker::NativeVideo;

#[cfg(not(windows))]
pub struct NativeVideo;
#[cfg(not(windows))]
impl NativeVideo {
    pub fn open(_: &Path, _: bool) -> Result<Self, String> {
        Err("native movie decoding backend is currently Windows-only".into())
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (0, 0)
    }
    pub fn duration_100ns(&self) -> i64 {
        0
    }
    #[cfg(test)]
    pub fn ended(&self) -> bool {
        true
    }
    pub fn frame_at(&mut self, _: i64) -> Result<Arc<Image>, String> {
        Err("movie backend unavailable".into())
    }
}
