//! One persistent output device; script-created Audio values own mixer voices.
#[path = "audio_output.rs"]
mod output;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use output::{Output, Voice};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

static DEVICE: OnceLock<Result<Arc<Output>, String>> = OnceLock::new();

/// Initialize before interactive playback without playing a sound.
pub fn warmup() -> Result<(), String> {
    device_output().map(|_| ())
}

/// Report asynchronous failures on the application thread without opening a device.
pub fn check_output() -> Result<(), String> {
    match DEVICE.get() {
        Some(Ok(output)) => output.check(),
        Some(Err(error)) => Err(error.clone()),
        None => Ok(()),
    }
}

fn device_output() -> Result<Arc<Output>, String> {
    let output = DEVICE
        .get_or_init(start_device)
        .as_ref()
        .map_err(Clone::clone)?;
    output.check()?;
    Ok(output.clone())
}

fn start_device() -> Result<Arc<Output>, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    // cpal::Stream need not be Send on every target. The owner thread constructs,
    // retains and destroys it; only the device-independent Output is shared.
    std::thread::Builder::new()
        .name("wolf-audio-device".into())
        .spawn(move || {
            let (stream, output) = match open_device() {
                Ok(value) => value,
                Err(error) => {
                    let _ = send.send(Err(error));
                    return;
                }
            };
            if send.send(Ok(output.clone())).is_err() {
                return;
            }
            while output.check().is_ok() {
                // Reclaim PCM away from the callback after it releases ownership.
                if output.reclaim().is_err() {
                    output.fail(output::ERROR_CONTROL);
                    break;
                }
                std::thread::park_timeout(Duration::from_millis(5));
            }
            drop(stream);
            let _ = output.retire_after_stream_stopped();
        })
        .map_err(|error| format!("audio device thread: {error}"))?;
    receive
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| format!("audio device initialization: {error}"))?
}

fn open_device() -> Result<(cpal::Stream, Arc<Output>), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output audio device")?;
    let supported = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let output = Arc::new(Output::new(supported.sample_rate())?);
    let format = supported.sample_format();
    let buffer_size = preferred_buffer_size(supported.buffer_size());
    let mut config: cpal::StreamConfig = supported.into();
    config.buffer_size = buffer_size;
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, output.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, output.clone()),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, output.clone()),
        _ => return Err("audio device sample format unsupported".into()),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    Ok((stream, output))
}

fn preferred_buffer_size(supported: &cpal::SupportedBufferSize) -> cpal::BufferSize {
    match *supported {
        cpal::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            cpal::BufferSize::Fixed(256_u32.clamp(min.max(1), max))
        }
        _ => cpal::BufferSize::Default,
    }
}

pub struct Audio {
    voice: Voice,
    pitch: u32,
    loop_start_ms: u32,
}
impl Audio {
    pub fn new(clip: wolf_media::Clip) -> Result<Self, String> {
        Self::with_options(clip, 64, true)
    }
    pub fn with_options(clip: wolf_media::Clip, gain: u16, looping: bool) -> Result<Self, String> {
        Self::with_position(clip, gain, looping, 0, 1)
    }
    /// Restore an elapsed source frame clock at the current device rate.
    pub fn with_position(
        clip: wolf_media::Clip,
        gain: u16,
        looping: bool,
        elapsed_frames: u64,
        elapsed_rate: u32,
    ) -> Result<Self, String> {
        Self::with_playback(clip, gain, looping, elapsed_frames, elapsed_rate, 100, 0)
    }
    pub fn with_playback(
        clip: wolf_media::Clip, gain: u16, looping: bool,
        elapsed_frames: u64, elapsed_rate: u32, pitch: u32, loop_start_ms: u32,
    ) -> Result<Self, String> {
        if elapsed_rate == 0 {
            return Err("zero saved audio sample rate".into());
        }
        if gain > 256 {
            return Err("audio gain exceeds mixer range".into());
        }
        let output = device_output()?;
        let clip = clip.resample_playback(output.sample_rate(), pitch)?;
        let loop_start = u64::from(loop_start_ms) * u64::from(output.sample_rate()) / (10 * u64::from(pitch));
        let loop_start = usize::try_from(loop_start).map_err(|_| "audio loop point overflow")?;
        Ok(Self {
            voice: output.start_with_loop(clip, gain, looping, elapsed_frames, elapsed_rate, loop_start)?,
            pitch,
            loop_start_ms,
        })
    }
    pub fn pitch(&self) -> u32 { self.pitch }
    pub fn loop_start_ms(&self) -> u32 { self.loop_start_ms }
    pub fn rendered_frames(&self) -> u64 {
        self.voice.rendered_frames()
    }
    pub fn sample_rate(&self) -> u32 {
        self.voice.sample_rate()
    }
    pub fn gain(&self) -> u16 {
        self.voice.gain()
    }
    pub fn pan(&self) -> i16 {
        self.voice.pan()
    }
    pub fn set_pan(&self, pan: i32) -> Result<(), String> {
        self.voice.set_pan(pan)
    }
    pub fn finished(&self) -> bool {
        self.voice.finished()
    }
    /// Logical fades publish gain; the device callback still owns the sample clock.
    pub fn set_gain(&self, gain: u16) -> Result<(), String> {
        self.voice.set_gain(gain)
    }
}
fn build<T: cpal::SizedSample + cpal::FromSample<i16>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    output: Arc<Output>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels);
    if channels == 0 {
        return Err("zero audio channels".into());
    }
    let errors = output.clone();
    let mut scratch = [[0i16; 2]; output::BLOCK_FRAMES];
    device
        .build_output_stream(
            config,
            move |device_buffer: &mut [T], _| {
                if !device_buffer.len().is_multiple_of(channels) {
                    output.fail(output::ERROR_BUFFER);
                    device_buffer.fill(T::from_sample(0));
                    return;
                }
                for block in device_buffer.chunks_mut(output::BLOCK_FRAMES * channels) {
                    let count = block.len() / channels;
                    output.render(&mut scratch[..count]);
                    for (frame, mixed) in block.chunks_exact_mut(channels).zip(&scratch[..count]) {
                        for (ch, sample) in frame.iter_mut().enumerate() {
                            let value = if channels == 1 {
                                ((i32::from(mixed[0]) + i32::from(mixed[1])) / 2) as i16
                            } else if ch < 2 {
                                mixed[ch]
                            } else {
                                0
                            };
                            *sample = T::from_sample(value);
                        }
                    }
                }
            },
            // No formatting/printing on the error callback; the app checks it.
            move |_| errors.fail(output::ERROR_DEVICE),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_request_respects_reported_device_buffer_limits() {
        use cpal::{BufferSize, SupportedBufferSize};
        for (min, max, expected) in [(64, 1024, 256), (512, 2048, 512), (32, 128, 128)] {
            assert_eq!(
                preferred_buffer_size(&SupportedBufferSize::Range { min, max }),
                BufferSize::Fixed(expected)
            );
        }
        assert_eq!(
            preferred_buffer_size(&SupportedBufferSize::Unknown),
            BufferSize::Default
        );
    }
}
