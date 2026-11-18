//! Platform initialization verified at A48E29/A48E33/A492C2 and 9EB45C.
use std::path::Path;
use wolf_vm::runtime::NativeContext;

pub struct Titles {
    title: String,
    plus: String,
    frame_skip: i32,
    window_mode: i32,
}
impl Titles {
    pub fn initialize(&self, vm: &mut wolf_vm::runtime::Vm) -> Result<(), String> {
        // 960B07/960B38 initializes system core+0x96C (sys25) to -1.
        vm.set_system(25, -1)?;
        // 960BBD initializes core+98C; 9E9C86 exposes it as sys11.
        vm.set_system(11, 1_000_000)?;
        vm.set_system(33, self.frame_skip)?;
        vm.set_system(138, self.window_mode)?;
        let language = system_language_name()?;
        vm.set_system(141, if language == "Japanese" { 0 } else { 1 })?;
        vm.set_system_text(76, language)?;
        vm.set_system_text(79, self.title.clone())?;
        vm.set_system_text(80, self.plus.clone())?;
        // 402295 -> 9605F0 -> 9613E0; 96153D/47/51 store 100.
        for slot in 100..=102 {
            vm.set_system(slot, 100)?;
        }
        // 9615A6/9615B0: diagonal X/Y walking speed percentages.
        vm.set_system(129, 100)?;
        vm.set_system(130, 100)?;
        // 960CCF..960D01 initializes keyboard codes and pad masks; 9BDC40
        // exposes the latter as public codes 301,302,303.
        for (slot, value) in [
            (52, 144),
            (53, 145),
            (54, 146),
            (55, 301),
            (56, 302),
            (57, 303),
        ] {
            vm.set_system(slot, value)?;
        }
        Ok(())
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        let bytes = crate::assets::read_member(root, "BasicData/Game.dat")?;
        let game =
            wolf_format::parse_game(&bytes, &Default::default()).map_err(|e| e.to_string())?;
        let ini = crate::window_config::read_ini(&root.join("Game.ini"))?;
        let window = crate::window_config::WindowConfig::from_words_and_ini(&game.unknown_words, &ini)?;
        let window_mode = match window.window_mode_flag {
            1 => 0,
            0 => 1,
            -1 => 2,
            value => return Err(format!("unsupported initial native window mode {value}")),
        };
        Ok(Self {
            title: game.title.text().map_err(|e| e.to_string())?.to_owned(),
            frame_skip: crate::window_config::frame_skip(&ini),
            window_mode,
            plus: game
                .extra_strings
                .first()
                .map(|s| s.text())
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or("")
                .to_owned(),
        })
    }
    pub fn execute(&self, ctx: &mut NativeContext<'_>) -> Result<bool, String> {
        let raw = ctx
            .instruction
            .raw
            .as_ref()
            .ok_or("missing native title record")?;
        if raw.opcode != 122 || raw.args.first() != Some(&9_900_060) {
            return Ok(false);
        }
        if raw.args != [9_900_060, 0, 0] || raw.strings.len() != 1 {
            return Err("unsupported system title assignment form".into());
        }
        let template = ctx.expand(raw.strings[0].text().map_err(|e| e.to_string())?)?;
        // This host's hardware-rendering path has no native software-mode suffix.
        let title =
            wolf_vm::native_values::expand_window_title(&template, &self.title, &self.plus, "")?;
        ctx.set_window_title(title)?;
        Ok(true)
    }
}

#[cfg(windows)]
pub fn system_language_name() -> Result<String, String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoA(locale: u32, kind: u32, buffer: *mut u8, length: i32) -> i32;
    }
    let mut buffer = [0u8; 256];
    let count = unsafe { GetLocaleInfoA(0x800, 0x1001, buffer.as_mut_ptr(), buffer.len() as i32) };
    let name = buffer.split(|&b| b == 0).next().unwrap_or(&[]);
    Ok(if count > 0 {
        String::from_utf8(name.to_vec()).map_err(|_| "English locale name is not ASCII")?
    } else {
        String::new()
    })
}
#[cfg(not(windows))]
pub fn system_language_name() -> Result<String, String> {
    Err("native system-language initialization needs a platform locale adapter".into())
}
