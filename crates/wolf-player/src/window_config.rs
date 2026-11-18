//! Game.dat display fields backed by native pointer binding at 00459460.
//! Logical resolution and presentation scale are deliberately separate.

use std::{
    io::{self, Read},
    path::Path,
};

/// Host resource policy, not a limit inferred from the original engine.
pub const MAX_DISPLAY_DIMENSION: u32 = 8192;
pub const MAX_RGBA_BYTES: usize = 64 * 1024 * 1024;
const MAX_INI_BYTES: usize = 1024 * 1024;

/// Native 9B8C4A..9B8DA5 treats an absent INI as empty text and uses key defaults.
/// Other I/O failures remain diagnostic errors in this host.
pub fn read_ini(path: &Path) -> Result<String, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_INI_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() > MAX_INI_BYTES {
        return Err("Game.ini exceeds the 1 MiB host input limit".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn checked_rgba_bytes(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_DISPLAY_DIMENSION || height > MAX_DISPLAY_DIMENSION
    {
        return Err(format!(
            "display dimensions {width}x{height} exceed host limit 1..={MAX_DISPLAY_DIMENSION}"
        ));
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("RGBA allocation size overflow")?;
    if bytes > MAX_RGBA_BYTES {
        return Err(format!(
            "display RGBA allocation {bytes} exceeds host limit {MAX_RGBA_BYTES}"
        ));
    }
    Ok(bytes)
}

/// 9B906C reads one character, missing key defaults to zero; 9BE390 clamps.
pub fn frame_skip(ini: &str) -> i32 {
    ini_integer(ini.split('\0').next().unwrap_or(""), "FrameSkip=", Some(1))
        .unwrap_or(0)
        .clamp(0, 2)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowConfig {
    pub logical_width: u32,
    pub logical_height: u32,
    /// Original INI value; mode 0 has a later native fullscreen transition.
    pub window_mode_flag: i32,
    pub display_number: i32,
    /// Requested presentation factor before the desktop-fit clamp.
    pub initial_scale: f64,
}

#[cfg(windows)]
pub fn native_fit_area() -> Option<(u32, u32)> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    // Same desktop-fit inputs as the A594D3/A594D9 caller of 93EDE0.
    let (width, height) = unsafe { (GetSystemMetrics(16), GetSystemMetrics(17)) };
    (width > 0 && height > 0).then_some((width as u32, height as u32))
}
#[cfg(not(windows))]
pub fn native_fit_area() -> Option<(u32, u32)> {
    None
}

#[cfg(windows)]
pub fn native_cursor_position() -> Option<(i32, i32)> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }
    let mut point = Point { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}
#[cfg(not(windows))]
pub fn native_cursor_position() -> Option<(i32, i32)> {
    None
}

impl WindowConfig {
    pub fn from_words_and_ini(words: &[u8], ini: &str) -> Result<Self, String> {
        // Original parser constructs its input through strlen at 9B8D34.
        let ini = ini.split('\0').next().unwrap_or("");
        let word = |index: usize| -> Result<i16, String> {
            let bytes = words
                .get(index * 2..index * 2 + 2)
                .ok_or_else(|| format!("Game.dat missing display word {index}"))?;
            Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
        };
        // 459773/45977C -> object+180/+184; A550F9..A55131 -> video size.
        let width = word(16)?;
        let height = word(17)?;
        if width <= 0 || height <= 0 {
            return Err(format!(
                "invalid Game.dat logical resolution {width}x{height}"
            ));
        }
        checked_rgba_bytes(width as u32, height as u32)?;
        // 9B9C95 defaults absent INI to -1. A592B5 tests <= 0, then
        // A59496 uses Game.dat word22, replacing zero with 1000.
        let ini_scale = ini_integer(ini, "Start_Window_Scale[x0.1]=", None).unwrap_or(-1);
        let units = if ini_scale > 0 {
            ini_scale
        } else {
            let configured = word(22)? as i32;
            if configured == 0 {
                1000
            } else {
                configured
            }
        };
        // Despite the INI key's label, A594BF multiplies by the f64 0.001
        // at CA7D90, then narrows to f32. 93EDE0 normalizes <=0 to 1.0.
        let scale = ((units as f64 * 0.001) as f32) as f64;
        Ok(Self {
            logical_width: width as u32,
            logical_height: height as u32,
            window_mode_flag: ini_integer(ini, "WindowModeFlag=", Some(2)).unwrap_or(1),
            display_number: ini_integer(ini, "Display_Number=", Some(1)).unwrap_or(0),
            initial_scale: if scale > 0.0 { scale } else { 1.0 },
        })
    }

    /// Positive Display_Number is one-based (A56D9D/A56DA3); zero is automatic.
    pub fn monitor_index(&self) -> Option<usize> {
        (self.display_number > 0).then(|| (self.display_number - 1) as usize)
    }

    /// Allocate only after dimension/byte checks; report allocator refusal.
    pub fn allocate_rgba(&self, color: [u8; 4]) -> Result<Vec<u8>, String> {
        let bytes = checked_rgba_bytes(self.logical_width, self.logical_height)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(bytes)
            .map_err(|error| format!("RGBA allocation failed: {error}"))?;
        result.resize(bytes, 0);
        for pixel in result.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        Ok(result)
    }

    /// Host conversion to positive physical pixels, retaining native float fit math.
    /// Exact original fractional pixel rounding is still a separate evidence gap.
    pub fn initial_client_size(&self, available: Option<(u32, u32)>) -> Result<(u32, u32), String> {
        checked_rgba_bytes(self.logical_width, self.logical_height)?;
        if !self.initial_scale.is_finite() || self.initial_scale <= 0.0 {
            return Err("invalid initial window scale".into());
        }
        let scale = available.map_or(self.initial_scale, |(w, h)| self.scale_fitting(w, h));
        if !scale.is_finite() || scale <= 0.0 {
            return Err("invalid initial window scale".into());
        }
        let width = (self.logical_width as f64 * scale).trunc().max(1.0);
        let height = (self.logical_height as f64 * scale).trunc().max(1.0);
        if width > MAX_DISPLAY_DIMENSION as f64 || height > MAX_DISPLAY_DIMENSION as f64 {
            return Err("scaled window dimensions exceed host limit".into());
        }
        let size = (width as u32, height as u32);
        checked_rgba_bytes(size.0, size.1)?;
        Ok(size)
    }

    /// The native caller uses GetSystemMetrics(16/17) here, not monitor bounds.
    /// Keep this separate so the platform layer supplies the actual fit area.
    pub fn scale_fitting(&self, available_width: u32, available_height: u32) -> f64 {
        let width_ratio = available_width as f32 / self.logical_width as f32;
        let height_ratio = available_height as f32 / self.logical_height as f32;
        let scale = (self.initial_scale as f32).min(width_ratio.min(height_ratio));
        if scale > 0.0 {
            scale as f64
        } else {
            1.0
        }
    }
}

// Native 9B8B30 searches the first exact token, then applies atoi. It does not
// parse INI sections or prefer the last occurrence. Preserve that behavior.
fn ini_integer(text: &str, key: &str, max_bytes: Option<usize>) -> Option<i32> {
    let start = text.find(key)? + key.len();
    let bytes = text.as_bytes().get(start..)?;
    let bytes = bytes.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
    let bytes = &bytes[..max_bytes.unwrap_or(bytes.len()).min(bytes.len())];
    let mut i = 0;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let negative = bytes.get(i) == Some(&b'-');
    if negative || bytes.get(i) == Some(&b'+') {
        i += 1;
    }
    let mut value = 0_i64;
    while let Some(digit) = bytes.get(i).filter(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + i64::from(*digit - b'0')).min(i32::MAX as i64 + 1);
        i += 1;
    }
    let value = if negative { -value } else { value };
    Some(value.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ini_frame_skip_reads_one_character_and_clamps_native_range() {
        for (ini, expected) in [
            ("", 0),
            ("FrameSkip=1", 1),
            ("FrameSkip=2", 2),
            ("FrameSkip=20", 2),
            ("FrameSkip=9", 2),
            ("FrameSkip=-1", 0),
            ("\0FrameSkip=2", 0),
        ] {
            assert_eq!(super::frame_skip(ini), expected);
        }
    }
    use super::*;

    fn words(width: i16, height: i16, scale: i16) -> Vec<u8> {
        let mut result = vec![0; 46];
        for (index, value) in [(16, width), (17, height), (22, scale)] {
            result[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        }
        result
    }

    #[test]
    fn configured_resolution_is_not_fixed_to_this_game() {
        let config = WindowConfig::from_words_and_ini(&words(854, 480, 1000), "").unwrap();
        assert_eq!((config.logical_width, config.logical_height), (854, 480));
        assert_eq!(config.initial_scale, 1.0);
        assert_eq!(config.window_mode_flag, 1);
    }

    #[test]
    fn nonpositive_ini_uses_game_scale_and_zero_game_scale_means_one() {
        for value in ["-1", "0"] {
            let ini = format!("Start_Window_Scale[x0.1]={value}\r\n");
            assert_eq!(
                WindowConfig::from_words_and_ini(&words(1280, 720, 1500), &ini)
                    .unwrap()
                    .initial_scale,
                1.5
            );
            assert_eq!(
                WindowConfig::from_words_and_ini(&words(1280, 720, 0), &ini)
                    .unwrap()
                    .initial_scale,
                1.0
            );
        }
    }

    #[test]
    fn positive_ini_uses_native_thousandths_and_fit_preserves_logical_size() {
        let config = WindowConfig::from_words_and_ini(
            &words(1600, 900, 1000),
            "WindowModeFlag=0\r\nDisplay_Number=2\r\nStart_Window_Scale[x0.1]=1500\r\n",
        )
        .unwrap();
        assert_eq!(config.initial_scale, 1.5);
        assert_eq!(config.scale_fitting(1200, 800), 0.75);
        assert_eq!((config.logical_width, config.logical_height), (1600, 900));
        assert_eq!((config.window_mode_flag, config.display_number), (0, 2));
    }

    #[test]
    fn missing_or_invalid_resolution_does_not_silently_use_a_fixed_size() {
        assert!(WindowConfig::from_words_and_ini(&[], "").is_err());
        assert!(WindowConfig::from_words_and_ini(&words(0, 720, 1000), "").is_err());
        assert!(WindowConfig::from_words_and_ini(&words(-1, 720, 1000), "").is_err());
    }

    #[test]
    fn resolution_limits_bound_each_dimension_and_total_rgba_allocation() {
        assert_eq!(checked_rgba_bytes(4096, 4096).unwrap(), MAX_RGBA_BYTES);
        assert!(checked_rgba_bytes(4096, 4097).is_err());
        assert!(checked_rgba_bytes(MAX_DISPLAY_DIMENSION + 1, 1).is_err());
        assert!(checked_rgba_bytes(u32::MAX, u32::MAX).is_err());
        assert!(WindowConfig::from_words_and_ini(&words(32767, 32767, 1000), "").is_err());
        let config = WindowConfig::from_words_and_ini(&words(3, 2, 1000), "").unwrap();
        let pixels = config.allocate_rgba([1, 2, 3, 255]).unwrap();
        assert_eq!(pixels, [1, 2, 3, 255].repeat(6));
    }

    #[test]
    fn presentation_size_is_bounded_even_without_a_monitor_and_never_zero() {
        let tiny = WindowConfig::from_words_and_ini(&words(320, 240, 1), "").unwrap();
        assert_eq!(tiny.initial_client_size(None).unwrap(), (1, 1));
        let huge = WindowConfig::from_words_and_ini(
            &words(1280, 720, 1000),
            "Start_Window_Scale[x0.1]=2147483647",
        )
        .unwrap();
        assert!(huge.initial_client_size(None).is_err());
        assert_eq!(
            huge.initial_client_size(Some((1280, 720))).unwrap(),
            (1280, 720)
        );
        let invalid = WindowConfig {
            initial_scale: f64::NAN,
            ..tiny
        };
        assert!(invalid.initial_client_size(None).is_err());
        assert!(invalid.initial_client_size(Some((1280, 720))).is_err());
    }

    #[test]
    fn monitor_number_is_one_based_and_native_token_widths_are_preserved() {
        let parse = |ini| WindowConfig::from_words_and_ini(&words(1280, 720, 1000), ini).unwrap();
        assert_eq!(parse("").monitor_index(), None);
        assert_eq!(parse("Display_Number=1").monitor_index(), Some(0));
        assert_eq!(parse("Display_Number=2").monitor_index(), Some(1));
        assert_eq!(parse("Display_Number=12").monitor_index(), Some(0));
        assert_eq!(parse("Display_Number=-1").monitor_index(), None);
        assert_eq!(parse("WindowModeFlag=-1").window_mode_flag, -1);
        assert_eq!(parse("WindowModeFlag=12").window_mode_flag, 12);
    }

    #[test]
    fn ini_first_token_nul_and_line_boundaries_match_native_read_shape() {
        let parse = |ini| WindowConfig::from_words_and_ini(&words(1280, 720, 1500), ini).unwrap();
        assert_eq!(
            parse("WindowModeFlag=1\nWindowModeFlag=0").window_mode_flag,
            1
        );
        assert_eq!(parse("\0WindowModeFlag=0").window_mode_flag, 1);
        assert_eq!(parse("Start_Window_Scale[x0.1]=\n2000").initial_scale, 1.5);
    }

    #[test]
    fn absent_ini_file_uses_defaults() {
        let path = std::env::temp_dir().join(format!(
            "wolf-absent-ini-{}-{}.ini",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!path.exists());
        let text = read_ini(&path).unwrap();
        let config = WindowConfig::from_words_and_ini(&words(1280, 720, 1000), &text).unwrap();
        assert_eq!(config.window_mode_flag, 1);
        assert_eq!(config.monitor_index(), None);
        assert_eq!(config.initial_scale, 1.0);
    }
}
