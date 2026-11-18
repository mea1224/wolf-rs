//! Fixed voice mailboxes and mixing, independent of an audio device.
//!
//! The publisher writes EMPTY then releases READY; the sole callback exclusively
//! mutates READY/PLAYING and releases DONE; a non-realtime reclaimer acquires DONE
//! and drops PCM. Stop/gain use atomics and cannot overflow a command queue.
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU8, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wolf_media::Clip;

pub(super) const BLOCK_FRAMES: usize = 1024;
const MAX_VOICES: usize = 128;
const MAX_PCM_BYTES: usize = 256 << 20;
const EMPTY: u8 = 0;
const READY: u8 = 1;
const PLAYING: u8 = 2;
const DONE: u8 = 3;
const NOT_STARTED: u64 = u64::MAX;
pub(super) const ERROR_DEVICE: u8 = 1;
pub(super) const ERROR_BUFFER: u8 = 2;
pub(super) const ERROR_CONTROL: u8 = 3;

struct Control {
    gain: AtomicU16,
    pan: AtomicI16,
    stopped: AtomicBool,
    started: AtomicU64,
}
struct PlayingVoice {
    clip: Clip,
    control: Arc<Control>,
    cursor: usize,
    looping: bool,
    loop_start: usize,
}
struct Slot {
    status: AtomicU8,
    value: UnsafeCell<MaybeUninit<PlayingVoice>>,
}
// SAFETY: Publishers/reclaimers serialize with Output::publisher. A consumer
// must own Output::rendering, which is never acquired by blocking/spinning.
// READY/PLAYING data belongs to that consumer; DONE data belongs to a reclaimer.
unsafe impl Sync for Slot {}
impl Drop for Slot {
    fn drop(&mut self) {
        if *self.status.get_mut() != EMPTY {
            // SAFETY: Output's last owner is gone, so no concurrent accessor
            // remains. Every non-empty slot contains an initialized voice.
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}
struct RenderGuard<'a>(&'a AtomicBool);
impl Drop for RenderGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct Output {
    sample_rate: u32,
    slots: Box<[Slot]>,
    /// Total published PCM, protected only on non-realtime threads.
    publisher: Mutex<usize>,
    pcm_budget: usize,
    rendered: AtomicU64,
    error: AtomicU8,
    /// CPAL serializes its FnMut callback. This nonblocking check also keeps
    /// the safe core API sound if a caller accidentally renders concurrently.
    rendering: AtomicBool,
}
impl Output {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        Self::with_pcm_budget(sample_rate, MAX_PCM_BYTES)
    }
    fn with_pcm_budget(sample_rate: u32, pcm_budget: usize) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("invalid output sample rate".into());
        }
        Ok(Self {
            sample_rate,
            slots: (0..MAX_VOICES)
                .map(|_| Slot {
                    status: AtomicU8::new(EMPTY),
                    value: UnsafeCell::new(MaybeUninit::uninit()),
                })
                .collect(),
            publisher: Mutex::new(0),
            pcm_budget,
            rendered: AtomicU64::new(0),
            error: AtomicU8::new(0),
            rendering: AtomicBool::new(false),
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn check(&self) -> Result<(), String> {
        match self.error.load(Ordering::Acquire) {
            0 => Ok(()),
            ERROR_DEVICE => Err("shared audio output device failed".into()),
            ERROR_BUFFER => Err("audio device returned an incomplete sample frame".into()),
            _ => Err("shared audio output control failed".into()),
        }
    }
    pub fn fail(&self, code: u8) {
        let _ = self
            .error
            .compare_exchange(0, code, Ordering::Release, Ordering::Relaxed);
    }
    #[cfg(test)]
    pub fn start(
        self: &Arc<Self>,
        clip: Clip,
        gain: u16,
        looping: bool,
        elapsed_frames: u64,
        elapsed_rate: u32,
    ) -> Result<Voice, String> {
        self.start_with_loop(clip, gain, looping, elapsed_frames, elapsed_rate, 0)
    }
    pub fn start_with_loop(
        self: &Arc<Self>, clip: Clip, gain: u16, looping: bool,
        elapsed_frames: u64, elapsed_rate: u32, loop_start: usize,
    ) -> Result<Voice, String> {
        self.check()?;
        if gain > 256 || clip.sample_rate() != self.sample_rate || elapsed_rate == 0 {
            return Err("invalid audio voice gain/sample rate".into());
        }
        let start_frame = u64::try_from(
            u128::from(elapsed_frames) * u128::from(self.sample_rate) / u128::from(elapsed_rate),
        )
        .map_err(|_| "saved audio frame clock overflow")?;
        let length = clip.frames().len();
        if loop_start >= length { return Err("audio loop point outside clip".into()); }
        let bytes = length
            .checked_mul(4)
            .ok_or("audio PCM byte count overflow")?;
        let control = Arc::new(Control {
            gain: AtomicU16::new(gain),
            pan: AtomicI16::new(0),
            stopped: AtomicBool::new(false),
            started: AtomicU64::new(NOT_STARTED),
        });
        let mut published = self
            .publisher
            .lock()
            .map_err(|_| "audio publisher poisoned")?;
        self.check()?;
        self.reclaim_locked(&mut published);
        let next_bytes = published
            .checked_add(bytes)
            .ok_or("audio PCM budget overflow")?;
        if next_bytes > self.pcm_budget {
            return Err(format!(
                "shared audio PCM exceeds {} byte budget",
                self.pcm_budget
            ));
        }
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.status.load(Ordering::Acquire) == EMPTY)
            .ok_or("shared audio voice limit (128); start was not queued")?;
        let voice = PlayingVoice {
            clip,
            control: control.clone(),
            cursor: if looping {
                if start_frame < length as u64 { start_frame as usize }
                else { loop_start + ((start_frame - length as u64) % (length - loop_start) as u64) as usize }
            } else {
                start_frame.min(length as u64) as usize
            },
            looping,
            loop_start,
        };
        // SAFETY: EMPTY belongs to the serialized publisher. The callback does
        // not inspect this value until it acquires the following READY store.
        unsafe { (*slot.value.get()).write(voice) };
        *published = next_bytes;
        slot.status.store(READY, Ordering::Release);
        Ok(Voice {
            output: self.clone(),
            control,
            start_frame,
            end_frame: (!looping).then_some(length as u64),
        })
    }
    pub fn reclaim(&self) -> Result<(), String> {
        let mut published = self
            .publisher
            .lock()
            .map_err(|_| "audio publisher poisoned")?;
        self.reclaim_locked(&mut published);
        Ok(())
    }
    fn reclaim_locked(&self, published: &mut usize) {
        for slot in &self.slots {
            if slot.status.load(Ordering::Acquire) == DONE {
                // SAFETY: DONE releases the consumer's ownership. The publisher
                // mutex prevents two reclaimers from reading this value.
                let voice = unsafe { (*slot.value.get()).assume_init_read() };
                *published -= voice.clip.frames().len() * 4;
                slot.status.store(EMPTY, Ordering::Release);
                drop(voice);
            }
        }
    }
    /// Call after destroying a failed device's stream. The guard additionally
    /// rejects any overlapping consumer, so misuse cannot free live PCM.
    pub fn retire_after_stream_stopped(&self) -> Result<(), String> {
        if self.error.load(Ordering::Acquire) == 0 {
            return Err("cannot retire a healthy audio output".into());
        }
        let mut published = self
            .publisher
            .lock()
            .map_err(|_| "audio publisher poisoned")?;
        let _guard = self.render_guard().ok_or("audio callback still active")?;
        for slot in &self.slots {
            if slot.status.load(Ordering::Acquire) != EMPTY {
                slot.status.store(DONE, Ordering::Release);
            }
        }
        self.reclaim_locked(&mut published);
        Ok(())
    }
    fn render_guard(&self) -> Option<RenderGuard<'_>> {
        self.rendering
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| RenderGuard(&self.rendering))
    }
    /// No allocation, reference-count changes, blocking lock, decoder or PCM
    /// destructor. Completed values remain in their slots for off-thread reclaim.
    pub fn render(&self, output: &mut [[i16; 2]]) {
        let Some(_guard) = self.render_guard() else {
            self.fail(ERROR_CONTROL);
            output.fill([0, 0]);
            return;
        };
        if self.error.load(Ordering::Acquire) != 0 {
            output.fill([0, 0]);
            return;
        }
        let mut sums = [[0i64; 2]; BLOCK_FRAMES];
        for block in output.chunks_mut(BLOCK_FRAMES) {
            let count = block.len();
            sums[..count].fill([0, 0]);
            let clock = self.rendered.load(Ordering::Relaxed);
            for slot in &self.slots {
                let status = slot.status.load(Ordering::Acquire);
                if status != READY && status != PLAYING {
                    continue;
                }
                let done = {
                    // SAFETY: READY/PLAYING belongs to the consumer holding the
                    // render guard. Producers/reclaimers never touch these data.
                    let voice = unsafe { (*slot.value.get()).assume_init_mut() };
                    if voice.control.stopped.load(Ordering::Acquire) {
                        true
                    } else {
                        if status == READY {
                            voice.control.started.store(clock, Ordering::Release);
                            slot.status.store(PLAYING, Ordering::Relaxed);
                        }
                        let gain = i64::from(voice.control.gain.load(Ordering::Relaxed));
                        let pan = i64::from(voice.control.pan.load(Ordering::Relaxed));
                        let left = 255 - pan.max(0);
                        let right = 255 + pan.min(0);
                        let pcm = voice.clip.frames();
                        for sum in &mut sums[..count] {
                            if voice.cursor == pcm.len() {
                                if voice.looping {
                                    voice.cursor = voice.loop_start;
                                } else {
                                    break;
                                }
                            }
                            let sample = pcm[voice.cursor];
                            sum[0] += i64::from(sample[0]) * gain * left / 255;
                            sum[1] += i64::from(sample[1]) * gain * right / 255;
                            voice.cursor += 1;
                        }
                        !voice.looping && voice.cursor == pcm.len()
                    }
                };
                if done {
                    // No access to voice or its references after this release:
                    // a non-realtime reclaimer may immediately destroy them.
                    slot.status.store(DONE, Ordering::Release);
                }
            }
            for (frame, sum) in block.iter_mut().zip(&sums[..count]) {
                *frame = sum.map(|value| {
                    (value / 256).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
                });
            }
            self.rendered.fetch_add(count as u64, Ordering::Release);
        }
    }
}

pub(super) struct Voice {
    output: Arc<Output>,
    control: Arc<Control>,
    start_frame: u64,
    end_frame: Option<u64>,
}
impl Voice {
    pub fn rendered_frames(&self) -> u64 {
        let started = self.control.started.load(Ordering::Acquire);
        let elapsed = if started == NOT_STARTED {
            0
        } else {
            self.output
                .rendered
                .load(Ordering::Acquire)
                .saturating_sub(started)
        };
        self.start_frame.saturating_add(elapsed)
    }
    pub fn sample_rate(&self) -> u32 {
        self.output.sample_rate
    }
    pub fn gain(&self) -> u16 {
        self.control.gain.load(Ordering::Relaxed)
    }
    pub fn pan(&self) -> i16 {
        self.control.pan.load(Ordering::Relaxed)
    }
    pub fn set_pan(&self, pan: i32) -> Result<(), String> {
        self.output.check()?;
        // 88CE90 / 88D870 clamp the native balance to -255..255.
        self.control.pan.store(pan.clamp(-255, 255) as i16, Ordering::Relaxed);
        Ok(())
    }
    pub fn finished(&self) -> bool {
        self.end_frame
            .is_some_and(|end| self.rendered_frames() >= end)
    }
    pub fn set_gain(&self, gain: u16) -> Result<(), String> {
        self.output.check()?;
        if gain > 256 {
            return Err("audio gain exceeds mixer range".into());
        }
        self.control.gain.store(gain, Ordering::Relaxed);
        Ok(())
    }
}
impl Drop for Voice {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pcm(values: &[[i16; 2]]) -> Clip {
        Clip::new(8000, values.to_vec()).unwrap()
    }
    fn output() -> Arc<Output> {
        Arc::new(Output::new(8000).unwrap())
    }
    #[test]
    fn native_pan_clamps_and_changes_channels_without_restarting_voice() {
        let output = output();
        let voice = output.start(pcm(&[[2550, 5100]; 8]), 256, true, 0, 1).unwrap();
        voice.set_pan(89).unwrap();
        let mut samples = [[0, 0]; 1];
        output.render(&mut samples);
        assert_eq!(samples, [[1660, 5100]]);
        voice.set_pan(-999).unwrap();
        output.render(&mut samples);
        assert_eq!(samples, [[2550, 0]]);
        assert_eq!(voice.rendered_frames(), 2);
        voice.set_pan(999).unwrap();
        output.render(&mut samples);
        assert_eq!(samples, [[0, 5100]]);
    }
    #[test]
    fn loop_metadata_preserves_intro_and_wraps_only_the_loop_region() {
        let output = output();
        let source = pcm(&[[1, 1], [2, 2], [3, 3], [4, 4]]);
        let voice = output.start_with_loop(source.clone(), 256, true, 0, 8000, 2).unwrap();
        let mut samples = [[0, 0]; 8];
        output.render(&mut samples);
        assert_eq!(samples.map(|frame| frame[0]), [1, 2, 3, 4, 3, 4, 3, 4]);
        drop(voice);
        let resumed = output.start_with_loop(source, 256, true, 5, 8000, 2).unwrap();
        output.render(&mut samples[..2]);
        assert_eq!(samples[..2], [[4, 4], [3, 3]]);
        assert_eq!(resumed.rendered_frames(), 7);
    }
    #[test]
    fn pitch_changes_playback_length_and_sample_selection() {
        let source = pcm(&[[1, 1], [2, 2], [3, 3], [4, 4]]);
        let fast = source.resample_playback(8000, 200).unwrap();
        assert_eq!(fast.frames(), [[1, 1], [3, 3]]);
        let slow = source.resample_playback(8000, 50).unwrap();
        assert_eq!(slow.frames(), [[1, 1], [1, 1], [2, 2], [2, 2], [3, 3], [3, 3], [4, 4], [4, 4]]);
    }

    #[test]
    fn voices_mix_before_clipping_and_gain_changes_do_not_restart() {
        let output = output();
        let first = output
            .start(pcm(&[[30000, -30000]; 4]), 256, false, 0, 1)
            .unwrap();
        let second = output
            .start(pcm(&[[-20000, 20000]; 4]), 256, false, 0, 1)
            .unwrap();
        let mut samples = [[0, 0]; 2];
        output.render(&mut samples);
        assert_eq!(samples, [[10000, -10000]; 2]);
        assert_eq!(first.rendered_frames(), 2);
        second.set_gain(128).unwrap();
        output.render(&mut samples);
        assert_eq!(samples, [[20000, -20000]; 2]);
        assert!(first.finished() && second.finished());
        assert_eq!(Arc::strong_count(&first.control), 2); // Callback did not free it.
        output.reclaim().unwrap();
        assert_eq!(Arc::strong_count(&first.control), 1);
        output.render(&mut samples);
        assert_eq!(samples, [[0, 0]; 2]);
        assert_eq!(first.rendered_frames(), 6);
    }

    #[test]
    fn saved_clock_converts_rate_and_loop_seek_wraps() {
        let output = output();
        let voice = output
            .start(pcm(&[[10, 20], [30, 40], [50, 60]]), 256, true, 10, 16000)
            .unwrap();
        assert_eq!(voice.rendered_frames(), 5);
        let mut samples = [[0, 0]; 4];
        output.render(&mut samples);
        assert_eq!(samples, [[50, 60], [10, 20], [30, 40], [50, 60]]);
        assert_eq!(voice.rendered_frames(), 9);
        assert!(!voice.finished());
        let ended = output
            .start(pcm(&[[1, 2]; 3]), 256, false, 50, 8000)
            .unwrap();
        assert!(ended.finished());
        drop(voice);
        output.render(&mut samples);
        assert_eq!(samples, [[0, 0]; 4]);
        assert_eq!(ended.rendered_frames(), 54);
    }

    #[test]
    fn drop_cancels_pending_and_active_without_stopping_other_voices() {
        let output = output();
        let pending = output
            .start(pcm(&[[100, 100]; 8]), 256, true, 0, 1)
            .unwrap();
        drop(pending);
        let active = output.start(pcm(&[[10, 20]; 8]), 256, true, 0, 1).unwrap();
        let survivor = output.start(pcm(&[[1, 2]; 8]), 256, true, 0, 1).unwrap();
        let mut samples = [[0, 0]; 2];
        output.render(&mut samples);
        assert_eq!(samples, [[11, 22]; 2]);
        drop(active);
        output.render(&mut samples);
        assert_eq!(samples, [[1, 2]; 2]);
        output.reclaim().unwrap();
        assert_eq!(*output.publisher.lock().unwrap(), 8 * 4);
        assert_eq!(survivor.rendered_frames(), 4);
    }

    #[test]
    fn bounded_publication_rejects_excess_and_reuses_only_retired_slots() {
        let output = output();
        let mut voices = Vec::new();
        for _ in 0..MAX_VOICES {
            voices.push(output.start(pcm(&[[1, 1]]), 256, true, 0, 1).unwrap());
        }
        assert!(output.start(pcm(&[[1, 1]]), 256, true, 0, 1).is_err());
        voices.pop();
        // Drop is a request, not permission to overwrite callback-owned PCM.
        assert!(output.start(pcm(&[[1, 1]]), 256, true, 0, 1).is_err());
        output.render(&mut [[0, 0]; 1]);
        let replacement = output.start(pcm(&[[7, 8]]), 256, true, 0, 1).unwrap();
        assert_eq!(replacement.rendered_frames(), 0);
        output.render(&mut [[0, 0]; 1]);
        assert_eq!(replacement.rendered_frames(), 1);
    }

    #[test]
    fn failure_is_explicit_and_callback_outputs_silence() {
        let output = output();
        let voice = output.start(pcm(&[[20, 30]]), 256, true, 0, 1).unwrap();
        output.fail(ERROR_DEVICE);
        output.fail(ERROR_BUFFER); // First failure remains visible.
        assert!(output.check().unwrap_err().contains("device"));
        assert!(voice.set_gain(128).is_err());
        assert!(output.start(pcm(&[[1, 1]]), 256, true, 0, 1).is_err());
        let mut samples = [[99, 99]; 2];
        output.render(&mut samples);
        assert_eq!(samples, [[0, 0]; 2]);
        output.retire_after_stream_stopped().unwrap();
        assert_eq!(*output.publisher.lock().unwrap(), 0);
    }

    #[test]
    fn pcm_budget_is_released_only_after_callback_retirement() {
        let output = Arc::new(Output::with_pcm_budget(8000, 64).unwrap());
        let first = output.start(pcm(&[[0, 0]; 8]), 256, true, 0, 1).unwrap();
        let _second = output.start(pcm(&[[0, 0]; 8]), 256, true, 0, 1).unwrap();
        assert!(
            output
                .start(pcm(&[[0, 0]; 8]), 256, true, 0, 1)
                .err()
                .unwrap()
                .contains("budget")
        );
        drop(first);
        assert!(output.start(pcm(&[[0, 0]; 8]), 256, true, 0, 1).is_err());
        output.render(&mut [[0, 0]; 1]);
        let replacement = output.start(pcm(&[[0, 0]; 8]), 256, true, 0, 1).unwrap();
        assert_eq!(*output.publisher.lock().unwrap(), 64);
        drop(replacement);
    }

    #[test]
    fn concurrent_publication_stop_and_reclamation_preserve_slot_ownership() {
        let output = output();
        let running = Arc::new(AtomicBool::new(true));
        let consumer_output = output.clone();
        let consumer_running = running.clone();
        let consumer = std::thread::spawn(move || {
            let mut block = [[0, 0]; 64];
            while consumer_running.load(Ordering::Acquire) {
                consumer_output.render(&mut block);
                std::thread::yield_now();
            }
            // Release any stop requests published before the producer finished.
            consumer_output.render(&mut block);
        });
        let mut completed = 0;
        for _ in 0..100_000 {
            output.reclaim().unwrap();
            match output.start(pcm(&[[7, -7]; 8]), 256, true, 0, 1) {
                Ok(voice) => {
                    voice.set_gain(128).unwrap();
                    drop(voice);
                    completed += 1;
                    if completed == 2000 {
                        break;
                    }
                }
                Err(error) => assert!(error.contains("voice limit"), "{error}"),
            }
            std::thread::yield_now();
        }
        running.store(false, Ordering::Release);
        consumer.join().unwrap();
        output.reclaim().unwrap();
        assert_eq!(completed, 2000);
        assert_eq!(*output.publisher.lock().unwrap(), 0);
        output.check().unwrap();
    }
}
