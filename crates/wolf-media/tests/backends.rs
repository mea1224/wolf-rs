use std::sync::Arc;
use wolf_media::{Clip, Image, Mixer, Picture, Pictures};

#[test]
fn composition_clips_scales_and_orders_transparency() {
    let red = Arc::new(Image::new(1, 1, vec![255, 0, 0, 255]).unwrap());
    let blue = Arc::new(Image::new(1, 1, vec![0, 0, 255, 128]).unwrap());
    let mut pictures = Pictures::default();
    pictures
        .show(
            2,
            Picture {
                image: blue,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                opacity: 255,
                z: 1,
            },
        )
        .unwrap();
    pictures
        .show(
            1,
            Picture {
                image: red,
                x: -1,
                y: 0,
                width: 3,
                height: 1,
                opacity: 255,
                z: 0,
            },
        )
        .unwrap();
    let mut target = Image::new(2, 1, vec![0; 8]).unwrap();
    pictures.render(&mut target);
    assert_eq!(target.bytes(), &[127, 0, 128, 255, 255, 0, 0, 255]);
    pictures.erase(2);
    let mut target = Image::new(2, 1, vec![0; 8]).unwrap();
    pictures.render(&mut target);
    assert_eq!(target.bytes(), &[255, 0, 0, 255, 255, 0, 0, 255]);
    assert!(Image::new(usize::MAX, 2, vec![]).is_err());
}

#[test]
fn audio_loop_pause_gain_clipping_and_completion_are_sample_exact() {
    let clip = Arc::new(Clip::new(48000, vec![[30000, -30000], [10000, -10000]]).unwrap());
    let mut mixer = Mixer::new(48000).unwrap();
    mixer.play(1, clip.clone(), 256, false).unwrap();
    mixer.play(2, clip.clone(), 256, true).unwrap();
    let mut output = [[0; 2]; 3];
    assert_eq!(mixer.render(&mut output).unwrap(), vec![1]);
    assert_eq!(output, [[32767, -32768], [20000, -20000], [30000, -30000]]);
    mixer.pause(2, true).unwrap();
    mixer.render(&mut output).unwrap();
    assert_eq!(output, [[0; 2]; 3]);
    mixer.pause(2, false).unwrap();
    mixer.gain(2, 128).unwrap();
    mixer.render(&mut output[..1]).unwrap();
    assert_eq!(output[0], [5000, -5000]);
    mixer.stop(2);
    mixer.render(&mut output).unwrap();
    assert_eq!(output, [[0; 2]; 3]);
    assert!(Mixer::new(44100)
        .unwrap()
        .play(1, clip, 256, false)
        .is_err());
}

#[test]
fn restored_audio_cursor_wraps_loops_and_completes_past_end() {
    let clip = Arc::new(Clip::new(48000, vec![[100, -100], [200, -200], [300, -300]]).unwrap());
    let mut mixer = Mixer::new(48000).unwrap();
    mixer.play(1, clip.clone(), 256, true).unwrap();
    mixer.seek(1, 8).unwrap();
    let mut output = [[0; 2]; 2];
    assert!(mixer.render(&mut output).unwrap().is_empty());
    assert_eq!(output, [[300, -300], [100, -100]]);
    mixer.stop(1);
    mixer.play(2, clip, 256, false).unwrap();
    mixer.seek(2, u64::MAX).unwrap();
    assert_eq!(mixer.render(&mut output).unwrap(), vec![2]);
    assert_eq!(output, [[0; 2]; 2]);
    assert!(mixer.seek(2, 0).is_err());
}
