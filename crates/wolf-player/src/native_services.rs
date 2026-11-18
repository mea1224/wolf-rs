//! Native host routing. Archive/decoder ownership stays outside deterministic VM state.
use crate::{
    native_media::{native_loop_name, MediaOutcome, NativeMedia},
    native_video::NativeVideo,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use wolf_assets::wolf350::{Archive, Limits};
use wolf_media::Image;
use wolf_vm::runtime::NativeContext;

struct Resources {
    root: PathBuf,
    save_root: PathBuf,
    archives: BTreeMap<String, Archive>,
    images: BTreeMap<String, Arc<Image>>,
    decoded_bytes: usize,
    archive_bytes: usize,
    font_families: Option<Vec<String>>,
    palette: Option<Vec<[u8; 3]>>,
    text_spacing: [i32; 2],
    glyphs: BTreeMap<(String, u32, u8, i32, u8, i32), crate::native_text::RasterText>,
}
impl Resources {
    fn font_families(&mut self) -> Result<Vec<String>, String> {
        if let Some(families) = &self.font_families {
            return Ok(families.clone());
        }
        let bytes = self.read("BasicData/Game.dat")?;
        let config = wolf_format::parse_game(&bytes, &wolf_format::Limits::default())
            .map_err(|e| e.to_string())?;
        let spacing = *config
            .configuration
            .get(11)
            .ok_or("text line spacing configuration missing")? as i8;
        let mode = *config
            .configuration
            .get(35)
            .ok_or("text spacing mode missing")? as i8;
        let scale = if mode > 0 {
            32
        } else if mode < 0 {
            0
        } else {
            i32::from(config.configuration[0].min(32))
        };
        self.text_spacing = [i32::from(spacing), scale];
        let families = std::iter::once(&config.font)
            .chain(config.fallback_fonts.iter())
            .map(|font| font.text().map(str::to_owned).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        self.font_families = Some(families.clone());
        Ok(families)
    }
    fn text_image(&mut self, text: &str) -> Result<Arc<Image>, String> {
        self.text_layout(text, false)?.image()
    }
    fn text_layout(&mut self, text: &str, progressive: bool) -> Result<crate::native_text::TextLayout, String> {
        let families = self.font_families()?;
        if self.palette.is_none() {
            let bytes = self.read("BasicData/SysDatabase.dat")?;
            let db = wolf_format::parse_database(&bytes, &wolf_format::Limits::default())
                .map_err(|e| e.to_string())?;
            let ty = db
                .value
                .types
                .get(12)
                .ok_or("system text color table missing")?;
            let mut colors = Vec::new();
            let mut row = 0;
            while let Some(red) = ty.integer(row, 0) {
                let green = ty.integer(row, 1).ok_or("text green channel missing")?;
                let blue = ty.integer(row, 2).ok_or("text blue channel missing")?;
                colors.push([
                    red.clamp(0, 255) as u8,
                    green.clamp(0, 255) as u8,
                    blue.clamp(0, 255) as u8,
                ]);
                row += 1;
            }
            self.palette = Some(colors);
        }
        let colors = self.palette.clone().unwrap();
        // Glyph rasterization is independent of reveal time. Keep a bounded
        // cache across text-picture replacements (dialogue and menu scripts).
        let mut glyphs = std::mem::take(&mut self.glyphs);
        let layout = if progressive { crate::native_text::layout_progressive } else { crate::native_text::layout_with_outline };
        let result = layout(
            text,
            28,
            self.text_spacing[0],
            self.text_spacing[1],
            |path: &str| self.image(path),
            |text: &str, size: u32, font: u8, color: i32, edge_width: u8, edge_color: i32| {
                let key = (text.to_owned(), size, font, color, edge_width, edge_color);
                if let Some(glyph) = glyphs.get(&key) { return Ok(glyph.clone()); }
                let family = families
                    .get(usize::from(font))
                    .ok_or("text font index missing")?;
                let last = colors.len().checked_sub(1).ok_or("text palette is empty")?;
                let color = &colors[(color.max(0) as usize).min(last)];
                let edge = usize::try_from(edge_color)
                    .ok()
                    .and_then(|i| colors.get(i))
                    .copied()
                    .unwrap_or([0; 3]);
                let glyph = crate::native_text_win::raster_outline(text, size, family, *color, edge_width, edge)?;
                if glyphs.len() >= 4096 { glyphs.clear(); }
                glyphs.insert(key, glyph.clone());
                Ok(glyph)
            },
        );
        self.glyphs = glyphs;
        result
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>, String> {
        // Script picture paths are relative to Data. Only the known sibling
        // Save mount escapes that asset directory, and keeps host isolation.
        if let Some(member) = path
            .strip_prefix("../Save/")
            .or_else(|| path.strip_prefix("..\\Save\\"))
        {
            let member = wolf_assets::normalize_path(member).map_err(|e| e.to_string())?;
            return crate::assets::read_file(&self.save_root.join(member), 64 << 20);
        }
        let path = wolf_assets::normalize_path(path).map_err(|e| e.to_string())?;
        let (mount, member) = path
            .split_once('/')
            .ok_or("media path requires Archive/member")?;
        if !self.archives.contains_key(mount) {
            let limits = Limits {
                member_bytes: 64 << 20,
                ..Limits::default()
            };
            let bytes = crate::assets::read_file(
                &self.root.join("Data").join(format!("{mount}.wolf")),
                limits.archive_bytes,
            )?;
            if self
                .archive_bytes
                .checked_add(bytes.len())
                .ok_or("archive cache size overflow")?
                > 512 << 20
            {
                return Err("native archive cache exceeds 512 MiB".into());
            }
            let length = bytes.len();
            let archive = Archive::open(bytes, limits).map_err(|e| e.to_string())?;
            self.archives.insert(mount.to_owned(), archive);
            self.archive_bytes += length;
        }
        Ok(self.archives[mount]
            .read(member)
            .map_err(|e| e.to_string())?
            .bytes
            .to_vec())
    }
    fn image(&mut self, path: &str) -> Result<Arc<Image>, String> {
        let writable = path.starts_with("../Save/") || path.starts_with("..\\Save\\");
        if let Some(image) = self.images.get(path).filter(|_| !writable) {
            return Ok(image.clone());
        }
        let bytes = self.read(path)?;
        let image =
            Arc::new(Image::from_encoded(&bytes).map_err(|e| format!("image {path}: {e}"))?);
        let new_size = self
            .decoded_bytes
            .checked_add(image.bytes().len())
            .ok_or("image cache size overflow")?;
        if new_size > 256 << 20 {
            return Err("native decoded-image cache exceeds 256 MiB".into());
        }
        if !writable {
            self.decoded_bytes = new_size;
            self.images.insert(path.to_owned(), image.clone());
        }
        Ok(image)
    }
    fn movie_file(&mut self, path: &str) -> Result<PathBuf, String> {
        let bytes = self.read(path)?;
        let digest = bytes.iter().fold(0xcbf29ce484222325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        let directory = self.root.join("wolf-rs/validation/media-cache");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let output = directory.join(format!("{digest:016x}.mp4"));
        // Cache identity includes all bytes; compare before reuse to avoid hash-collision aliases.
        if !output.exists() || crate::assets::read_file(&output, 64 << 20)? != bytes {
            std::fs::write(&output, bytes).map_err(|e| e.to_string())?;
        }
        Ok(output)
    }
}
struct Movie {
    path: String,
    video: NativeVideo,
    started: Option<u64>,
}

#[path = "native_services_state.rs"]
mod state;
#[path = "native_screenshot.rs"]
mod screenshot;

pub struct Services {
    resources: Resources,
    media: NativeMedia,
    movies: BTreeMap<i32, Movie>,
    ticks: u64,
    audio: Option<crate::native_audio::NativeAudio>,
}
impl Services {
    pub(crate) fn input_font(&mut self) -> Result<String, String> {
        self.resources.font_families()?.into_iter().next().ok_or("game input font missing".into())
    }
    pub(crate) fn set_save_root(&mut self, path: &Path) {
        self.resources.save_root = path.to_owned();
    }
    pub(crate) fn read_resource(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.resources.read(path)
    }
    pub(crate) fn image_resource(&mut self, path: &str) -> Result<Arc<Image>, String> {
        self.resources.image(path)
    }
    pub fn new(root: &Path, width: u32, height: u32) -> Result<Self, String> {
        let resources = Resources {
            root: root.to_owned(),
            save_root: root.join("Save"),
            archives: BTreeMap::new(),
            images: BTreeMap::new(),
            decoded_bytes: 0,
            archive_bytes: 0,
            font_families: None,
            palette: None,
            text_spacing: [0, 32],
            glyphs: BTreeMap::new(),
        };
        Ok(Self {
            resources,
            media: NativeMedia::new(width, height)?,
            movies: BTreeMap::new(),
            ticks: 0,
            audio: None,
        })
    }
    pub fn vm_blocked(&self) -> bool {
        self.media.vm_blocked()
    }
    pub(crate) fn map_transition(&mut self, flags: i32) -> Result<bool, String> {
        self.media.map_transition(flags)
    }
    pub fn set_camera_offset(&mut self, offset: [i32; 2]) {
        self.media.set_camera_offset(offset);
    }
    pub fn set_scene_image(&mut self, image: Arc<Image>) -> Result<(), String> {
        self.media.set_scene_image(image)
    }
    pub fn sync_bgm_master(&mut self, master: i32, bgs_master: i32) -> Result<(), String> {
        if let Some(audio) = &mut self.audio {
            audio.sync_bgm_master(master)?;
            audio.sync_bgs_master(bgs_master)?;
        }
        Ok(())
    }
    pub(crate) fn route_sound(
        &mut self,
        row: i32,
        world: &wolf_vm::runtime::World,
    ) -> Result<(), String> {
        use wolf_vm::native_values::{DatabaseAddress, DatabaseKind};
        self.initialize_audio()?;
        self.audio
            .as_mut()
            .expect("initialized audio service")
            .route_effect(
                row,
                *world.systems.numbers.get(&102).unwrap_or(&100),
                |type_id, row, field| {
                    world
                        .databases
                        .get(DatabaseAddress {
                            kind: DatabaseKind::System,
                            type_id,
                            row,
                            field,
                        })
                        .cloned()
                },
                |path| self.resources.read(path),
            )
    }
    pub(crate) fn map_audio(&mut self, kind: u8, row: i32, path: Option<String>, world: &wolf_vm::runtime::World) -> Result<(), String> {
        use wolf_vm::native_values::{DatabaseAddress, DatabaseKind};
        if row == -2 { return Ok(()); }
        self.initialize_audio()?;
        self.audio.as_mut().expect("initialized audio service").map_change(
            kind, row, path,
            *world.systems.numbers.get(&(100 + i32::from(kind))).unwrap_or(&100),
            |type_id, row, field| world.databases.get(DatabaseAddress { kind: DatabaseKind::System, type_id, row, field }).cloned(),
            |path| self.resources.read(path),
        )
    }
    fn initialize_audio(&mut self) -> Result<(), String> {
        if self.audio.is_none() {
            let game_bytes = self.resources.read("BasicData/Game.dat")?;
            let config = wolf_format::parse_game(&game_bytes, &wolf_format::Limits::default())
                .map_err(|e| e.to_string())?;
            self.audio = Some(crate::native_audio::NativeAudio::new(
                &config.configuration,
            )?);
        }
        Ok(())
    }
    pub fn execute(&mut self, context: &mut NativeContext<'_>) -> Result<bool, String> {
        let command = context
            .instruction
            .raw
            .as_ref()
            .ok_or("native host instruction lacks raw command")?;
        let mut args = command.args.clone();
        match command.opcode {
            1000 => {
                if args.len() != 1 || !matches!(args[0], 0 | 2) {
                    return Err(format!("unsupported native file operation {args:?}"));
                }
                let expected = if args[0] == 0 { 1 } else { 2 };
                if command.strings.len() != expected {
                    return Err("1000 screenshot/copy path count".into());
                }
                let paths = command.strings.iter().map(|text| {
                    let expanded = context.expand(text.text().map_err(|e| e.to_string())?)?;
                    screenshot::storage_path(&self.resources.root, &self.resources.save_root, &expanded)
                }).collect::<Result<Vec<_>, String>>()?;
                // A2D9F0 mode0 captures immediately with JPEG quality85;
                // mode2 expands both paths then CopyFileW(..., false).
                if args[0] == 0 {
                    screenshot::write_jpeg(&self.media.render()?, &paths[0])?;
                } else {
                    if let Some(parent) = paths[1].parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::copy(&paths[0], &paths[1])
                        .map_err(|e| format!("1000 copy {} to {}: {e}", paths[0].display(), paths[1].display()))?;
                }
                Ok(true)
            }
            124 => {
                // A14C10 category4 routes picture selectors to 993B90. Its
                // selector9 returns zero for an absent picture; a fading delete
                // remains present until its timer expires (993B90 case9).
                if args.len() != 4 || !matches!(args[1], 0x4000 | 0x4040) || !matches!(args[3], 0..=3 | 9 | 10) || args[1] == 0x4040 && args[3] > 1 {
                    return Err(format!("unsupported native picture query {args:?}"));
                }
                let id = context.number(args[2])?;
                let value = match (args[3], self.media.picture_dimensions(id)) {
                    (axis @ (0 | 1), _) => (if args[1] & 0x40 != 0 {
                        self.media.picture_effect_position(id)?
                    } else { self.media.picture_position(id)? }).map_or(-1, |p| p[axis as usize]),
                    (2, Some((width, _))) => width as i32,
                    (3, Some((_, height))) => height as i32,
                    (9, _) => i32::from(self.media.picture_exists(id)),
                    (10, _) => self.media.picture_text_finished(id),
                    _ => -1,
                };
                context.set_number(args[0], value)?;
                Ok(true)
            }
            150 => {
                for (i, value) in args.iter_mut().enumerate().skip(1) {
                    if !matches!(i, 11 | 12) {
                        *value = context.number(*value)?;
                    }
                }
                let flags = *args.first().ok_or("150 missing flags")? as u32;
                let operation = flags & 15;
                let variant = (flags >> 4) & 15;
                let text = if operation == 0 {
                    match variant {
                        0 | 2 | 3 => context.expand(
                            command
                                .strings
                                .first()
                                .ok_or("150 missing resource string")?
                                .text()
                                .map_err(|e| e.to_string())?,
                        )?,
                        1 => context.expand(
                            &context.text(
                                *args
                                    .get(11)
                                    .ok_or("150 variable path lacks string address")?,
                            )?,
                        )?,
                        other => {
                            return Err(format!(
                                "150 content variant {other} needs a dedicated native renderer"
                            ))
                        }
                    }
                } else {
                    String::new()
                };
                let id = if operation == 0 {
                    *args.get(1).ok_or("150 missing picture ID")?
                } else {
                    0
                }; // Only create owns a video decoder; media resolves ranges.
                let mut movie = None;
                let text_image = if operation == 0 && variant == 2 {
                    let layout = self.resources.text_layout(&text, true)?;
                    Some(crate::native_media::text::TextReveal {
                        width: layout.width, height: layout.height,
                        glyphs: layout.images.into_iter().map(|g| crate::native_media::text::Glyph {
                            image: g.image, x: g.x, y: g.y, reveal_at: g.reveal_at,
                        }).collect(),
                        units: layout.reveal_units, instant_ranges: layout.instant_ranges,
                        rate: context.number(9_000_011)?, accumulator: 0.0, last_step: 0, visible: 0,
                    })
                } else {
                    None
                };
                if operation == 0 && variant != 2 && text.to_ascii_lowercase().ends_with(".mp4") {
                    if self.movies.len() >= 8 && !self.movies.contains_key(&id) {
                        return Err("simultaneous native movie limit".into());
                    }
                    let file = self.resources.movie_file(&text)?;
                    let mut video = NativeVideo::open(&file, native_loop_name(&text))?;
                    println!(
                        "NATIVE MOVIE id={id} path={text} dimensions={:?} duration_100ns={}",
                        video.dimensions(),
                        video.duration_100ns()
                    );
                    let first = video.frame_at(0)?;
                    movie = Some((video, first));
                }
                let result = if let Some(layout) = text_image {
                    self.media.execute_text(&command.args, &args, &text, layout)?
                } else {
                    self.media
                        .execute(command.opcode, &command.args, &args, &text, |path| {
                            if let Some((_, first)) = &movie {
                                Ok(first.clone())
                            } else {
                                self.resources.image(path)
                            }
                        })?
                };
                if result == MediaOutcome::Applied && operation == 0 {
                    if let Some((video, _)) = movie {
                        self.movies.insert(
                            id,
                            Movie {
                                path: text,
                                video,
                                started: None,
                            },
                        );
                    } else {
                        self.movies.remove(&id);
                    }
                }
                Ok(result == MediaOutcome::Applied)
            }
            151 => {
                if !command.strings.is_empty() { return Err("151 does not accept strings".into()); }
                for value in args.iter_mut().skip(1) { *value = context.number(*value)?; }
                Ok(self.media.execute(151, &command.args, &args, "", |_| Err("151 cannot load images".into()))? == MediaOutcome::Applied)
            }
            290 => {
                let screen_mask = args.first().is_some_and(|flags| flags & 15 == 3);
                if !screen_mask && !command.strings.is_empty() {
                    return Err("290 picture effect does not accept strings".into());
                }
                let path = if screen_mask && matches!((args[0] >> 4) & 255, 5 | 6) {
                    if command.strings.len() != 1 {
                        return Err("290 mask load requires one path".into());
                    }
                    context.expand(command.strings[0].text().map_err(|e| e.to_string())?)?
                } else {
                    String::new()
                };
                // A2BF3F..A2BFE5 resolves every operand except the packed flags.
                for value in args.iter_mut().skip(1) {
                    *value = context.number(*value)?;
                }
                Ok(self
                    .media
                    .execute(290, &command.args, &args, &path, |path| {
                        self.resources.image(path)
                    })?
                    == MediaOutcome::Applied)
            }
            160..=162 => {
                if command.opcode == 160 && args.len() > 2 {
                    args[2] = context.number(args[2])?;
                }
                let path = if command.opcode == 160 && args.first().is_some_and(|kind| *kind >= 0) {
                    use wolf_vm::native_values::{DatabaseAddress, DatabaseKind, NativeValue};
                    match context.world.databases.get(DatabaseAddress {
                        kind: DatabaseKind::System,
                        type_id: 11,
                        row: args[0] as usize,
                        field: 0,
                    })? {
                        NativeValue::Text(path) => path.clone(),
                        _ => return Err("160 SystemDB mask path is not text".into()),
                    }
                } else {
                    String::new()
                };
                Ok(self
                    .media
                    .execute(command.opcode, &command.args, &args, &path, |path| {
                        self.resources.image(path)
                    })?
                    == MediaOutcome::Applied)
            }
            140 => {
                self.initialize_audio()?;
                self.audio
                    .as_mut()
                    .expect("initialized audio service")
                    .execute(context, |path| self.resources.read(path))?;
                Ok(true)
            }
            opcode => Err(format!("native services does not route opcode {opcode}")),
        }
    }
    pub fn tick(&mut self) -> Result<(), String> {
        self.ticks = self
            .ticks
            .checked_add(1)
            .ok_or("native service tick overflow")?;
        self.media.tick(1)?;
        let mut expired = Vec::new();
        for (&id, movie) in &mut self.movies {
            match self.media.picture_path(id) {
                Some(path) if path == movie.path => {
                    let started = *movie.started.get_or_insert(self.ticks);
                    let elapsed = i64::try_from((self.ticks - started) as u128 * 10_000_000 / 60)
                        .map_err(|_| "movie presentation timestamp overflow")?;
                    self.media
                        .replace_frame(id, movie.video.frame_at(elapsed)?)?;
                }
                _ if movie.started.is_some() => expired.push(id),
                _ => (), // delayed create has not become visible yet
            }
        }
        for id in expired {
            self.movies.remove(&id);
        }
        if let Some(audio) = &mut self.audio {
            audio.tick()?;
        }
        Ok(())
    }
    pub fn render(&mut self) -> Result<Image, String> {
        self.media.render()
    }
    #[cfg(test)]
    pub fn active_picture_paths(&self) -> Vec<(i32, String)> {
        self.media.active_paths()
    }
}
