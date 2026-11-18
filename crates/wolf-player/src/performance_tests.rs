//! CPU timing only; window presentation is measured separately by the player.
use std::{path::Path, time::Instant};

#[test]
#[ignore = "private archive checkpoint CPU composition measurement"]
fn local_checkpoint_cpu_performance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let checkpoint = std::env::var_os("WOLF_PERFORMANCE_CHECKPOINT")
        .expect("set WOLF_PERFORMANCE_CHECKPOINT to a real input-driven checkpoint");
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let destination = root.join("wolf-rs/validation").join(format!("cpu-performance-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(destination.join("Save")).unwrap();
    let mut game = crate::game::Game::startup(root).unwrap();
    game.set_save_root(&destination.join("Save")).unwrap();
    game.diagnostic_restore_checkpoint(Path::new(&checkpoint)).unwrap();
    let mut metrics = crate::frame_metrics::FrameMetrics::default();
    for frame in 0..660 {
        let tick = Instant::now();
        game.tick().unwrap();
        let updated = Instant::now();
        let image = game.render().unwrap();
        if frame >= 60 {
            metrics.record("logic_update", updated - tick);
            metrics.record("composition", updated.elapsed());
            metrics.record("cpu_frame", tick.elapsed());
        }
        if frame == 659 {
            image.write_png(std::fs::File::create(destination.join("last.png")).unwrap()).unwrap();
        }
    }
    let report = metrics.report();
    std::fs::write(destination.join("timings.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!("CPU_ONLY {}\n{report}", destination.display());
}
