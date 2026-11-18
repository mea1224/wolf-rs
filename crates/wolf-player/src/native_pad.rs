//! WinMM ordinal buttons to the native DirectInput ordinal mask (784AAF..784B35).
//! Axis calibration and XInput-specific trigger synthesis remain separate capabilities.
#[cfg(windows)]
pub fn sample_device() -> (u32, bool) {
    use std::{sync::{Arc, OnceLock, atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering}}, time::{Duration, Instant}};
    use windows::Win32::Media::Multimedia::{
        joyGetDevCapsW, joyGetNumDevs, joyGetPosEx, JOYCAPSW, JOYINFOEX,
    };
    struct Snapshot { connected: AtomicBool, buttons: AtomicU32, updated_ms: AtomicU64, epoch: Instant }
    static SNAPSHOT: OnceLock<Arc<Snapshot>> = OnceLock::new();
    let snapshot = SNAPSHOT.get_or_init(|| {
        let snapshot = Arc::new(Snapshot { connected: AtomicBool::new(false), buttons: AtomicU32::new(0), updated_ms: AtomicU64::new(0), epoch: Instant::now() });
        let output = snapshot.clone();
        // Device enumeration/drivers can block. They never run on the input,
        // VM or presentation thread. Rediscovery also supports hot-plugging.
        if let Err(error) = std::thread::Builder::new().name("wolf-gamepad".into()).spawn(move || {
            let mut device = None;
            let mut next_scan = Instant::now();
            loop {
                if device.is_none() && Instant::now() >= next_scan {
                    for id in 0..unsafe { joyGetNumDevs() }.min(256) {
                        let mut caps = JOYCAPSW::default();
                        if unsafe { joyGetDevCapsW(id as usize, &mut caps, size_of::<JOYCAPSW>() as u32) } != 0 || caps.wNumButtons == 0 { continue; }
                        let mut state = JOYINFOEX { dwSize: size_of::<JOYINFOEX>() as u32, dwFlags: 0x80, ..Default::default() };
                        if unsafe { joyGetPosEx(id, &mut state) } == 0 { device = Some((id, caps.wNumButtons.min(15))); break; }
                    }
                    next_scan = Instant::now() + Duration::from_secs(1);
                }
                let mut buttons = 0;
                if let Some((id, count)) = device {
                    let mut state = JOYINFOEX { dwSize: size_of::<JOYINFOEX>() as u32, dwFlags: 0x80, ..Default::default() };
                    if unsafe { joyGetPosEx(id, &mut state) } == 0 { buttons = (state.dwButtons & ((1 << count) - 1)) << 4; }
                    else { device = None; }
                }
                output.connected.store(device.is_some(), Ordering::Relaxed);
                output.buttons.store(buttons, Ordering::Relaxed);
                output.updated_ms.store(output.epoch.elapsed().as_millis() as u64, Ordering::Release);
                std::thread::sleep(Duration::from_millis(4));
            }
        }) { eprintln!("gamepad polling unavailable: {error}"); }
        snapshot
    });
    if (snapshot.epoch.elapsed().as_millis() as u64).saturating_sub(snapshot.updated_ms.load(Ordering::Acquire)) > 100 {
        return (0, false); // A blocked/disconnected driver must not hold stale buttons.
    }
    (snapshot.buttons.load(Ordering::Relaxed), snapshot.connected.load(Ordering::Relaxed))
}

#[cfg(not(windows))]
pub fn sample_device() -> (u32, bool) {
    (0, false)
}
