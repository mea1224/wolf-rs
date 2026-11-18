//! Decode ahead on the COM owner's thread. A bounded channel retains at most
//! two upcoming frames; presentation still selects by the script's exact PTS.
use super::{platform::Decoder, Image};
use std::{path::{Path, PathBuf}, sync::{Arc, mpsc::{self, Receiver}}};

enum Frame { Image(i64, Arc<Image>), End, Error(String) }

pub struct NativeVideo {
    path: PathBuf,
    looping: bool,
    size: (u32, u32),
    duration: i64,
    frames: Receiver<Frame>,
    pending: Option<(i64, Arc<Image>)>,
    current: Option<Arc<Image>>,
    previous_time: i64,
    ended: bool,
}
impl NativeVideo {
    pub fn open(path: &Path, looping: bool) -> Result<Self, String> {
        let path = path.to_owned();
        let source = path.clone();
        let (hello_tx, hello_rx) = mpsc::sync_channel(1);
        let (tx, frames) = mpsc::sync_channel(2);
        std::thread::Builder::new().name("wolf-video-decode".into()).spawn(move || {
            let mut decoder = match Decoder::open(&source) {
                Ok(decoder) => decoder,
                Err(error) => { let _ = hello_tx.send(Err(error)); return; }
            };
            let duration = decoder.duration_100ns();
            if hello_tx.send(Ok((decoder.dimensions(), duration))).is_err() { return; }
            let mut epoch = 0_i64;
            let mut empty = 0;
            loop {
                match decoder.read_frame() {
                    Ok(Some((pts, image))) => {
                        empty = 0;
                        let Some(pts) = epoch.checked_add(pts) else {
                            let _ = tx.send(Frame::Error("video PTS overflow".into())); return;
                        };
                        if tx.send(Frame::Image(pts, image)).is_err() { return; }
                    }
                    Ok(None) => { empty += 1; }
                    Err(error) => { let _ = tx.send(Frame::Error(error)); return; }
                }
                if decoder.ended {
                    if !looping { let _ = tx.send(Frame::End); return; }
                    let Some(next) = epoch.checked_add(duration) else {
                        let _ = tx.send(Frame::Error("video loop clock overflow".into())); return;
                    };
                    epoch = next;
                    if let Err(error) = decoder.seek_zero() {
                        let _ = tx.send(Frame::Error(error)); return;
                    }
                }
                if empty > 4096 {
                    let _ = tx.send(Frame::Error("video produced no frames within sample budget".into())); return;
                }
            }
        }).map_err(|e| format!("video worker: {e}"))?;
        let (size, duration) = hello_rx.recv().map_err(|e| format!("video initialization: {e}"))??;
        Ok(Self { path, looping, size, duration, frames, pending: None, current: None, previous_time: -1, ended: false })
    }
    pub fn dimensions(&self) -> (u32, u32) { self.size }
    pub fn duration_100ns(&self) -> i64 { self.duration }
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn ended(&self) -> bool { self.ended }

    pub fn frame_at(&mut self, elapsed: i64) -> Result<Arc<Image>, String> {
        if elapsed < 0 { return Err("negative video elapsed time".into()); }
        if elapsed < self.previous_time {
            // Drop disconnects the old bounded channel and releases its decoder
            // on its owner thread. No COM pointer crosses threads.
            *self = Self::open(&self.path, self.looping)?;
        }
        self.previous_time = elapsed;
        for _ in 0..4096 {
            if let Some((pts, _)) = self.pending.as_ref() {
                if *pts > elapsed && self.current.is_some() {
                    return Ok(self.current.as_ref().unwrap().clone());
                }
                self.current = self.pending.take().map(|(_, image)| image);
            }
            if self.ended {
                return self.current.clone().ok_or("video produced no frames".into());
            }
            match self.frames.recv().map_err(|e| format!("video decoder disconnected: {e}"))? {
                Frame::Image(pts, image) => self.pending = Some((pts, image)),
                Frame::End => self.ended = true,
                Frame::Error(error) => return Err(error),
            }
        }
        Err("video frame advance exceeded sample budget".into())
    }
}
