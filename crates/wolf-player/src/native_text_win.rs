//! Windows font raster callback. Text and font family come from game data;
//! native layout/control handling stays in native_text. Exact DX font edge
//! filtering remains subject to screenshot comparison.
use super::native_text::RasterText;

/// GDI coverage plus a bounded CPU outline. Width/color follow native text
/// controls; the DX font backend's exact edge filter remains a visual parity
/// boundary, as with the base glyph rasterization.
pub fn raster_outline(text:&str,size:u32,family:&str,color:[u8;3],edge_width:u8,edge:[u8;3])->Result<RasterText,String>{
    let mut raster=raster_color(text,size,family,color)?;
    if edge_width==0 {return Ok(raster);}
    if edge_width>9 {return Err("text outline width exceeds native bound".into());}
    let (w,h)=raster.image.dimensions();
    let radius=i32::from(edge_width);
    let source=raster.image.bytes();
    let mut pixels=source.to_vec();
    for y in 0..h as i32 { for x in 0..w as i32 {
        let mut coverage=0u32;
        for oy in -radius..=radius { for ox in -radius..=radius {
            if ox*ox+oy*oy>radius*radius {continue;}
            let px=x+ox;let py=y+oy;
            if px>=0 && py>=0 && px<w as i32 && py<h as i32 {
                coverage=coverage.max(u32::from(source[((py as u32*w+px as u32)*4+3) as usize]));
            }
        }}
        let i=((y as u32*w+x as u32)*4) as usize;
        let a=u32::from(source[i+3]);
        let combined=a*255+coverage*(255-a);
        if combined==0 {continue;}
        for c in 0..3 {pixels[i+c]=((u32::from(color[c])*a*255+u32::from(edge[c])*coverage*(255-a)+combined/2)/combined) as u8;}
        pixels[i+3]=((combined+127)/255) as u8;
    }}
    raster.image=std::sync::Arc::new(wolf_media::Image::new(w as usize,h as usize,pixels)?);
    Ok(raster)
}

pub fn raster_color(text: &str, size: u32, family: &str, color: [u8;3]) -> Result<RasterText, String> {
    let mut raster = raster(text,size,family)?;
    let (width,height) = raster.image.dimensions();
    let mut pixels = raster.image.bytes().to_vec();
    for pixel in pixels.chunks_exact_mut(4) { pixel[..3].copy_from_slice(&color); }
    raster.image = std::sync::Arc::new(wolf_media::Image::new(width as usize,height as usize,pixels)?);
    Ok(raster)
}

#[cfg(not(windows))]
pub fn raster(_: &str, _: u32, _: &str) -> Result<RasterText, String> {
    Err("native text font rasterization is not implemented on this platform".into())
}

#[cfg(windows)]
pub fn raster(text: &str, size: u32, family: &str) -> Result<RasterText, String> {
    use std::{ffi::c_void, ptr, sync::Arc};
    use wolf_media::Image;
    type Handle = *mut c_void;
    #[repr(C)]
    struct Size {
        cx: i32,
        cy: i32,
    }
    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    }
    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 1],
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateCompatibleDC(dc: Handle) -> Handle;
        fn DeleteDC(dc: Handle) -> i32;
        fn CreateFontW(
            height: i32,
            width: i32,
            escapement: i32,
            orientation: i32,
            weight: i32,
            italic: u32,
            underline: u32,
            strike_out: u32,
            char_set: u32,
            out_precision: u32,
            clip_precision: u32,
            quality: u32,
            pitch_family: u32,
            face: *const u16,
        ) -> Handle;
        fn SelectObject(dc: Handle, object: Handle) -> Handle;
        fn DeleteObject(object: Handle) -> i32;
        fn GetTextExtentPoint32W(dc: Handle, text: *const u16, count: i32, size: *mut Size) -> i32;
        fn CreateDIBSection(
            dc: Handle,
            info: *const BitmapInfo,
            usage: u32,
            bits: *mut *mut c_void,
            section: Handle,
            offset: u32,
        ) -> Handle;
        fn SetTextColor(dc: Handle, color: u32) -> u32;
        fn SetBkColor(dc: Handle, color: u32) -> u32;
        fn TextOutW(dc: Handle, x: i32, y: i32, text: *const u16, count: i32) -> i32;
        fn GdiFlush() -> i32;
    }
    struct Dc(Handle);
    impl Drop for Dc {
        fn drop(&mut self) {
            unsafe {
                DeleteDC(self.0);
            }
        }
    }
    struct Selected {
        dc: Handle,
        object: Handle,
        previous: Handle,
    }
    impl Drop for Selected {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.object);
            }
        }
    }
    unsafe fn select(dc: Handle, object: Handle) -> Result<Selected, String> {
        if object.is_null() {
            return Err("GDI object creation failed".into());
        }
        let previous = unsafe { SelectObject(dc, object) };
        if previous.is_null() || previous as isize == -1 {
            unsafe {
                DeleteObject(object);
            }
            return Err("GDI object selection failed".into());
        }
        Ok(Selected {
            dc,
            object,
            previous,
        })
    }
    if text.len() > 1 << 20 || text.contains('\0') || family.contains('\0') || family.len() > 1024 {
        return Err("native font input byte limit or embedded NUL".into());
    }
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let family: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
    let count = i32::try_from(utf16.len()).map_err(|_| "font input length overflow")?;
    unsafe {
        let dc = Dc(CreateCompatibleDC(ptr::null_mut()));
        if dc.0.is_null() {
            return Err("GDI memory DC creation failed".into());
        }
        // ANTIALIASED_QUALITY: grayscale coverage avoids subpixel color fringes
        // when the resulting RGBA image is scaled/composited by the picture engine.
        let _font = select(
            dc.0,
            CreateFontW(
                -(size.clamp(2, 999) as i32),
                0,
                0,
                0,
                // 753AF0 stores thickness; 8B3276 multiplies it by 100.
                // The original startup display-state probe records thickness 5.
                500,
                0,
                0,
                0,
                1,
                0,
                0,
                4,
                0,
                family.as_ptr(),
            ),
        )?;
        let mut extent = Size { cx: 0, cy: 0 };
        if GetTextExtentPoint32W(dc.0, utf16.as_ptr(), count, &mut extent) == 0 {
            return Err("GDI text measurement failed".into());
        }
        let width = extent.cx.max(1);
        let height = extent.cy.max(1);
        if width > 32768 || height > 32768 || i64::from(width) * i64::from(height) > 16_000_000 {
            return Err("native font raster pixel budget".into());
        }
        let info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width,
                height: -height,
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc.0, &info, 0, &mut bits, ptr::null_mut(), 0);
        let _bitmap = select(dc.0, bitmap)?;
        if bits.is_null() {
            return Err("GDI DIB has no pixel memory".into());
        }
        let length = width as usize * height as usize * 4;
        ptr::write_bytes(bits as *mut u8, 0, length);
        SetTextColor(dc.0, 0xFFFFFF);
        SetBkColor(dc.0, 0);
        if TextOutW(dc.0, 0, 0, utf16.as_ptr(), count) == 0 || GdiFlush() == 0 {
            return Err("GDI text rasterization failed".into());
        }
        let bgra = std::slice::from_raw_parts(bits as *const u8, length);
        let mut rgba = Vec::with_capacity(length);
        for pixel in bgra.chunks_exact(4) {
            rgba.extend_from_slice(&[255, 255, 255, pixel[0].max(pixel[1]).max(pixel[2])]);
        }
        Ok(RasterText {
            image: Arc::new(Image::new(width as usize, height as usize, rgba)?),
            // 974600/9746A9 use normalized font size, not GDI cell height,
            // for rich-text row height and bottom alignment.
            advance: extent.cx,
            line_height: size.clamp(2, 999),
        })
    }
}
