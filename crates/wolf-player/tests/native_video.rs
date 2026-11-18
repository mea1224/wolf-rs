#![cfg(windows)]
#[path = "../src/native_video.rs"]
mod native_video;
use native_video::NativeVideo;

#[test]
fn real_title_pts_loop_and_decoder_cleanup() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../validation/archive-reference/Picture/title_loop.mp4");
    if !fixture.exists() {
        eprintln!("private movie fixture absent: {}", fixture.display());
        return;
    }
    for _ in 0..2 {
        let mut video = NativeVideo::open(&fixture, true).unwrap();
        assert_eq!(video.dimensions(), (1280, 720));
        let first = video.frame_at(0).unwrap();
        let later = video.frame_at(5_000_000).unwrap();
        assert_ne!(first.bytes(), later.bytes());
        let replay = video.frame_at(video.duration_100ns()).unwrap();
        assert_eq!(first.bytes(), replay.bytes());
        assert!(!video.ended());
        let output =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../validation/native-media");
        std::fs::create_dir_all(&output).unwrap();
        first
            .write_png(std::fs::File::create(output.join("title-video-0ms.png")).unwrap())
            .unwrap();
        later
            .write_png(std::fs::File::create(output.join("title-video-500ms.png")).unwrap())
            .unwrap();
    }
}
