//! Candidate media state format; independent of native .sav serialization.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SavedServices {
    version: u32,
    media: serde_json::Value,
    movies: Vec<(i32, String, Option<u64>)>,
    ticks: u64,
    audio: Option<serde_json::Value>,
}

impl Services {
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let state = SavedServices {
            version: 2,
            media: serde_json::from_slice(&self.media.capture_state()?)
                .map_err(|e| format!("media snapshot JSON: {e}"))?,
            movies: self
                .movies
                .iter()
                .map(|(&id, movie)| (id, movie.path.clone(), movie.started))
                .collect(),
            ticks: self.ticks,
            audio: self
                .audio
                .as_ref()
                .map(|audio| {
                    serde_json::from_slice(&audio.capture_state()?)
                        .map_err(|e| format!("audio snapshot JSON: {e}"))
                })
                .transpose()?,
        };
        let mut bytes = crate::native_media::snapshot_bytes::BoundedBytes::new(512 << 20);
        serde_json::to_writer(&mut bytes, &state).map_err(|e| format!("services snapshot: {e}"))?;
        Ok(bytes.bytes)
    }
    #[cfg(test)]
    #[allow(dead_code)] // Used by the separate native_services integration harness.
    pub fn restore(root: &Path, bytes: &[u8]) -> Result<Self, String> {
        Self::restore_with_save_root(root, None, bytes)
    }
    pub fn restore_with_save_root(
        root: &Path,
        save_root: Option<&Path>,
        bytes: &[u8],
    ) -> Result<Self, String> {
        if bytes.len() > 768 << 20 {
            return Err("services snapshot exceeds host budget".into());
        }
        let state: SavedServices =
            serde_json::from_slice(bytes).map_err(|e| format!("services snapshot: {e}"))?;
        if state.version != 2 || state.movies.len() > 8 {
            return Err("unsupported services snapshot version/movie count".into());
        }
        let media = NativeMedia::restore_state(
            &serde_json::to_vec(&state.media).map_err(|e| format!("media snapshot JSON: {e}"))?,
        )?;
        let (width, height) = media.viewport();
        let mut services = Self::new(root, width, height)?;
        if let Some(save_root) = save_root {
            services.set_save_root(save_root);
        }
        services.media = media;
        services.ticks = state.ticks;
        for (id, path, started) in state.movies {
            if started.is_some_and(|tick| tick > state.ticks) || services.movies.contains_key(&id) {
                return Err("invalid saved movie time/duplicate ID".into());
            }
            let file = services.resources.movie_file(&path)?;
            let mut video = NativeVideo::open(&file, native_loop_name(&path))?;
            if let Some(started) = started {
                let elapsed = i64::try_from(u128::from(state.ticks - started) * 10_000_000 / 60)
                    .map_err(|_| "saved movie timestamp overflow")?;
                // Decode to the saved timestamp before the next tick; the saved picture
                // bitmap preserves the exact last render boundary during restoration.
                video.frame_at(elapsed)?;
            }
            services.movies.insert(
                id,
                Movie {
                    path,
                    video,
                    started,
                },
            );
        }
        if let Some(value) = state.audio {
            let game_bytes = services.resources.read("BasicData/Game.dat")?;
            let config = wolf_format::parse_game(&game_bytes, &wolf_format::Limits::default())
                .map_err(|e| e.to_string())?;
            let mut audio = crate::native_audio::NativeAudio::new(&config.configuration)?;
            let bytes =
                serde_json::to_vec(&value).map_err(|e| format!("audio snapshot JSON: {e}"))?;
            audio.restore_state(&bytes, |path| services.resources.read(path))?;
            services.audio = Some(audio);
        }
        Ok(services)
    }
    pub fn restore_native(
        &mut self,
        snapshot: &crate::native_save_payload::SaveSnapshot,
    ) -> Result<(), String> {
        let (width, height) = self.media.viewport();
        let mut media = NativeMedia::new(width, height)?;
        media.restore_native_screen(&snapshot.system_fields)?;
        media.set_camera_offset(self.media.camera_offset());
        for picture in &snapshot.pictures {
            let text_image = if picture.fields.get(&764) == Some(&1) {
                Some(
                    self.resources
                        .text_image(&crate::native_text::decode_saved_text(&picture.source)?)?,
                )
            } else {
                None
            };
            media.restore_native_picture(
                &picture.fields,
                &picture.source,
                &picture.effect_arrays,
                |path| {
                    if let Some(image) = &text_image {
                        Ok(image.clone())
                    } else {
                        self.resources.image(path)
                    }
                },
            )?;
        }
        let game_bytes = self.resources.read("BasicData/Game.dat")?;
        let config = wolf_format::parse_game(&game_bytes, &wolf_format::Limits::default())
            .map_err(|e| e.to_string())?;
        let mut audio = crate::native_audio::NativeAudio::new(&config.configuration)?;
        audio.restore_native(snapshot, |path| self.resources.read(path))?;
        self.media = media;
        self.movies.clear();
        self.ticks = 0;
        self.audio = Some(audio);
        Ok(())
    }
}
