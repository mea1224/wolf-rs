mod assets;
mod audio;
mod display;
mod game;
mod keyboard;
mod native_audio;
mod native_control;
mod native_media;
mod native_map_audio;
mod native_save_cipher;
mod native_save_payload;
mod native_scene;
mod native_scene_render;
mod native_actor_render;
mod native_tile_render;
mod native_services;
mod native_system;
mod native_text;
mod native_text_win;
mod native_video;
mod playback_clock;
mod frame_metrics;
#[cfg(test)]
mod performance_tests;
mod window_config;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};
use wolf_media::Image;

fn draw_diagnostic(image: &mut Image, message: Option<&str>) {
    let (width, height) = image.dimensions();
    let bytes = image.bytes_mut();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[25, 29, 34, 255]);
    }
    // A diagnostic marker is deliberately not a map renderer.
    for y in 48..56.min(height as usize) {
        for x in 32..(width as usize).min(928) {
            let p = (y * width as usize + x) * 4;
            bytes[p..p + 4].copy_from_slice(&[205, 150, 54, 255]);
        }
    }
    let _ = message;
}

struct App {
    root: PathBuf,
    window_config: window_config::WindowConfig,
    image: Image,
    display: Option<display::Display>,
    window: Option<Arc<Window>>,
    audio_path: Option<PathBuf>,
    audio: Option<audio::Audio>,
    audio_started: Instant,
    last_frame: Instant,
    count: u64,
    error: Option<String>,
    capture: Option<PathBuf>,
    capture_done: bool,
    smoke_duration: Duration,
    presented: u64,
    game: Option<game::Game>,
    messages: Vec<String>,
    announced_message: Option<String>,
    clock: Option<playback_clock::PlaybackClock>,
    tick_count: u64,
    max_tick_lag: Duration,
    stopped: bool,
    keyboard: keyboard::Keyboard,
    input_probe: bool,
    capture_tick: u64,
    probe_press_tick: Option<u64>,
    startup_confirm_tick: Option<u64>,
    last_vm_status: String,
    performance_report: Option<PathBuf>,
    metrics: frame_metrics::FrameMetrics,
    previous_present: Option<Instant>,
    input_received: Option<Instant>,
    sampled_input: Option<Instant>,
    trace_vm: bool,
    applied_title: Option<String>,
    applied_window_mode: Option<i32>,
}
impl App {
    fn apply_window_mode(&mut self) -> Result<(), String> {
        let Some(window) = &self.window else { return Ok(()); };
        let mode = self.game.as_ref().and_then(|game| game.vm.state().world.systems.numbers.get(&138)).copied().unwrap_or(1 - self.window_config.window_mode_flag);
        if self.applied_window_mode == Some(mode) { return Ok(()); }
        let monitor = window.current_monitor();
        match mode {
            0 => {
                window.set_fullscreen(None);
                window.set_decorations(true);
                let size = self.window_config.initial_client_size(window_config::native_fit_area())?;
                let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1));
            }
            1 => window.set_fullscreen(Some(Fullscreen::Borderless(monitor))),
            2 => {
                // Host mapping of native ChangeWindowMode(0). Keep this distinct
                // from the custom borderless mode, without claiming identical DX behavior.
                let monitor = monitor.ok_or("fullscreen display unavailable")?;
                let size = monitor.size();
                let video = monitor.video_modes().min_by_key(|mode| {
                    let candidate = mode.size();
                    (candidate.width.abs_diff(size.width) + candidate.height.abs_diff(size.height), std::cmp::Reverse(mode.refresh_rate_millihertz()), std::cmp::Reverse(mode.bit_depth()))
                }).ok_or("fullscreen display has no video modes")?;
                window.set_fullscreen(Some(Fullscreen::Exclusive(video)));
            }
            _ => return Err(format!("invalid window mode {mode}")),
        }
        self.applied_window_mode = Some(mode);
        Ok(())
    }

    fn load(root: PathBuf, options: &Options) -> Result<Self, String> {
        let loaded = if options.media_probe {
            game::Game::media_probe(&root)
        } else if options.input_probe {
            game::Game::input_probe(&root)
        } else if let Some(program) = options.program.as_deref() {
            println!("DIAGNOSTIC EVENT ENTRY: {program}; automatic page selection is bypassed explicitly");
            game::Game::load(&root, None, program)
        } else {
            game::Game::startup(&root)
        };
        let (mut game, startup_error) = match loaded {
            Ok(game) => (Some(game), None),
            Err(error) => (None, Some(error)),
        };
        if let (Some(game), Some(save_root)) = (&mut game, &options.save_root) {
            game.set_save_root(save_root)?;
        }
        if let Some(checkpoint) = &options.restore_checkpoint {
            let game = game.as_mut().ok_or("checkpoint restore requires a successful default startup")?;
            game.diagnostic_restore_checkpoint(checkpoint)?;
            println!("DIAGNOSTIC CHECKPOINT {} restored at VM tick {} map {}", checkpoint.display(), game.vm.state().tick, game.map_name);
        }
        if let Some(error) = &startup_error {
            eprintln!("game startup unavailable: {error}");
        }
        let config_asset = assets::read_member(&root, "BasicData/Game.dat")?;
        let config = wolf_format::parse_game(&config_asset, &wolf_format::Limits::default())
            .map_err(|e| e.to_string())?;
        let ini = window_config::read_ini(&root.join("Game.ini"))?;
        let window_config =
            window_config::WindowConfig::from_words_and_ini(&config.unknown_words, &ini)?;
        let (width, height) = (window_config.logical_width, window_config.logical_height);
        println!(
            "Game.dat logical resolution {width}x{height}; configured initial scale {}",
            window_config.initial_scale
        );
        let mut image = Image::new(
            width as usize,
            height as usize,
            window_config.allocate_rgba([16, 20, 28, 255])?,
        )?;
        draw_diagnostic(&mut image, None);
        if let Some(game) = &game {
            println!("Loaded real Data/MapData.wolf/{}: {}x{}, {} map events, {} pages/programs, corpus {} instructions ({} lowered, {} unsupported)", game.map_name, game.map.width, game.map.height, game.map.events.len(), game.map_programs, game.coverage.total, game.coverage.lowered, game.coverage.unsupported.values().sum::<usize>());
        }
        Ok(Self {
            root,
            window_config,
            image,
            display: None,
            window: None,
            audio_path: options.audio.clone(),
            audio: None,
            audio_started: Instant::now(),
            last_frame: Instant::now(),
            count: 0,
            error: startup_error.clone(),
            capture: options.capture.clone(),
            capture_done: false,
            smoke_duration: options.smoke,
            presented: 0,
            game,
            messages: startup_error.into_iter().collect(),
            announced_message: None,
            clock: None,
            tick_count: 0,
            max_tick_lag: Duration::ZERO,
            stopped: false,
            keyboard: keyboard::Keyboard::default(),
            input_probe: options.input_probe,
            capture_tick: options.capture_tick,
            probe_press_tick: options.probe_press_tick,
            startup_confirm_tick: options.startup_confirm_tick,
            last_vm_status: String::new(),
            performance_report: options.performance_report.clone(),
            metrics: frame_metrics::FrameMetrics::default(),
            previous_present: None,
            input_received: None,
            sampled_input: None,
            trace_vm: options.trace_vm,
            applied_title: None,
            applied_window_mode: None,
        })
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = if self.messages.is_empty() {
            "wolf-rs - diagnostic execution - close window to exit"
        } else {
            "wolf-rs - startup unresolved; details in console - close window to exit"
        };
        let (width, height) = (
            self.window_config.logical_width,
            self.window_config.logical_height,
        );
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let monitor = if let Some(index) = self.window_config.monitor_index() {
            let Some(monitor) = monitors.get(index).cloned() else {
                self.error = Some(format!(
                    "configured Display_Number {} is unavailable ({} displays)",
                    self.window_config.display_number,
                    monitors.len()
                ));
                event_loop.exit();
                return;
            };
            Some(monitor)
        } else {
            window_config::native_cursor_position()
                .and_then(|(x, y)| {
                    monitors
                        .iter()
                        .min_by_key(|monitor| {
                            let position = monitor.position();
                            let size = monitor.size();
                            let left = i64::from(position.x);
                            let top = i64::from(position.y);
                            let right = left + i64::from(size.width.saturating_sub(1));
                            let bottom = top + i64::from(size.height.saturating_sub(1));
                            let dx = i128::from(i64::from(x) - i64::from(x).clamp(left, right));
                            let dy = i128::from(i64::from(y) - i64::from(y).clamp(top, bottom));
                            dx * dx + dy * dy
                        })
                        .cloned()
                })
                .or_else(|| event_loop.primary_monitor())
                .or_else(|| monitors.first().cloned())
        };
        let fit = window_config::native_fit_area().or_else(|| {
            monitor.as_ref().map(|m| {
                let s = m.size();
                (s.width, s.height)
            })
        });
        let client = match self.window_config.initial_client_size(fit) {
            Ok(size) => size,
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
        };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::PhysicalSize::new(client.0, client.1));
        match self.window_config.window_mode_flag {
            0 => {
                // Native 943260 is a custom borderless path; winit reproduces its
                // display-sized presentation, not each native style transition.
                attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(monitor.clone())));
            }
            1 | -1 => (),
            flag => {
                self.error = Some(format!("unsupported WindowModeFlag={flag}"));
                event_loop.exit();
                return;
            }
        }
        match event_loop.create_window(attrs) {
            Ok(window) => {
                if self.window_config.window_mode_flag == 1 {
                    if let Some(monitor) = &monitor {
                        let position = monitor.position();
                        let area = monitor.size();
                        let outer = window.outer_size();
                        let x = i64::from(position.x)
                            + (i64::from(area.width) - i64::from(outer.width)) / 2;
                        let y = i64::from(position.y)
                            + (i64::from(area.height) - i64::from(outer.height)) / 2;
                        window.set_outer_position(winit::dpi::PhysicalPosition::new(
                            x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                            y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                        ));
                    }
                }
                let window = Arc::new(window);
                match futures_lite::future::block_on(display::Display::new(
                    window.clone(),
                    (width, height),
                )) {
                    Ok(display) => {
                        self.display = Some(display);
                        self.window = Some(window);
                        self.applied_window_mode = (self.window_config.window_mode_flag != -1).then_some(1 - self.window_config.window_mode_flag);
                        if let Err(error) = audio::warmup() {
                            self.error = Some(error);
                            event_loop.exit();
                            return;
                        }
                        let ready = Instant::now();
                        self.clock = Some(playback_clock::PlaybackClock::start(
                            ready,
                            self.smoke_duration,
                        ));
                        self.last_frame = ready;
                        self.audio_started = ready;
                        println!("playback clock started after display initialization; target 60 logical ticks/s");
                    }
                    Err(error) => {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                event_loop.exit();
            }
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => self.keyboard.focused(focused),
            WindowEvent::Resized(size) => {
                if let Some(display) = &mut self.display {
                    display.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(game) = self.game.as_mut().filter(|game| game.text_input_active()) {
                    self.keyboard.release_all();
                    if event.state == ElementState::Pressed {
                        game.text_input_key(&event.logical_key);
                        if let Some(text) = &event.text { game.text_input_commit(text); }
                    }
                    return;
                }
                if !event.repeat && self.performance_report.is_some() {
                    self.input_received.get_or_insert_with(Instant::now);
                }
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.keyboard.key(key, event.state == ElementState::Pressed);
                }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(event.logical_key, Key::Named(NamedKey::Space))
                {
                    if let Some(game) = &mut self.game {
                        if let Err(e) = game.acknowledge_messages() {
                            self.error = Some(e);
                            event_loop.exit();
                        }
                    }
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                if let Some(game) = &mut self.game { game.text_input_commit(&text); }
            }
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                if let Some(game) = &mut self.game { game.text_input_preedit(&text); }
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = &self.window {
                    window.set_ime_allowed(self.game.as_ref().is_some_and(|game| game.text_input_active()));
                }
                let compose_started = Instant::now();
                if self.messages.is_empty() && !self.input_probe {
                    if let Some(game) = &mut self.game {
                        match game.render() {
                            Ok(image) => self.image = image,
                            Err(e) => {
                                self.error = Some(e);
                                event_loop.exit();
                                return;
                            }
                        }
                    }
                } else {
                    draw_diagnostic(&mut self.image, self.messages.last().map(String::as_str));
                }
                if self.performance_report.is_some() { self.metrics.record("composition", compose_started.elapsed()); }
                if let Some(display) = &mut self.display {
                    let capture_ready = self
                        .game
                        .as_ref()
                        .is_none_or(|g| g.vm.state().tick >= self.capture_tick)
                        && self.probe_press_tick.is_none_or(|tick| {
                            self.game.as_ref().is_some_and(|g| g.vm.state().tick > tick)
                        });
                    let capture = if self.capture_done || !capture_ready {
                        None
                    } else {
                        self.capture.as_deref()
                    };
                    let present_started = Instant::now();
                    match display.render(&self.image, capture) {
                        Ok(true) => {
                            if self.performance_report.is_some() {
                                let now = Instant::now();
                                self.metrics.record("upload_submit_present", present_started.elapsed());
                                if let Some(previous) = self.previous_present.replace(now) { self.metrics.record("present_interval", now - previous); }
                                if let Some(input) = self.sampled_input.take() { self.metrics.record("input_event_to_present_submit", now - input); }
                            }
                            self.presented += 1;
                            self.count += 1;
                            if capture.is_some() {
                                self.capture_done = true
                            }
                        }
                        Ok(false) => (),
                        Err(e) => {
                            self.error = Some(e);
                            event_loop.exit();
                        }
                    }
                }
                if self.last_frame.elapsed() >= Duration::from_secs(1) {
                    println!(
                        "displayed {} frames; {} logical updates in {:.3}s; maximum tick lag {:.3}ms",
                        self.count,
                        self.tick_count,
                        self.last_frame.elapsed().as_secs_f64(),
                        self.max_tick_lag.as_secs_f64() * 1000.0,
                    );
                    if let Some(audio) = &self.audio {
                        println!("audio output frames {}", audio.rendered_frames());
                    }
                    self.count = 0;
                    self.tick_count = 0;
                    self.max_tick_lag = Duration::ZERO;
                    self.last_frame = Instant::now();
                }
                if self.audio.is_none() && self.audio_started.elapsed() >= Duration::from_secs(2) {
                    if let Some(path) = &self.audio_path {
                        match assets::read_member(&self.root, &path.to_string_lossy())
                            .and_then(|b| {
                                if path
                                    .extension()
                                    .is_some_and(|e| e.eq_ignore_ascii_case("ogg"))
                                {
                                    wolf_media::Clip::from_ogg(&b)
                                } else {
                                    wolf_media::Clip::from_wav(&b)
                                }
                            })
                            .and_then(audio::Audio::new)
                        {
                            Ok(audio) => {
                                println!("audio device started: {}", path.display());
                                self.audio = Some(audio)
                            }
                            Err(e) => {
                                self.error = Some(e);
                                event_loop.exit();
                            }
                        }
                    }
                }
            }
            _ => (),
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.stopped {
            return;
        }
        if let Err(error) = audio::check_output() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        let Some(clock) = self.clock.as_ref() else {
            return;
        };
        if clock.deadline.is_some_and(|end| Instant::now() >= end) {
            if self.probe_press_tick.is_some()
                && self.game.as_ref().is_none_or(|g| {
                    g.vm.state().world.commons.numbers.get(&2658) != Some(&130)
                        || !g
                            .vm
                            .state()
                            .fibers
                            .values()
                            .all(|f| f.wait == wolf_vm::runtime::Wait::Done)
                })
            {
                self.error = Some(
                    "automated native keyboard probe did not complete with cself58=130".into(),
                );
            }
            if self.presented == 0
                || (self.capture.is_some() && !self.capture_done)
                || (self.audio_path.is_some()
                    && self.audio.as_ref().is_none_or(|a| a.rendered_frames() == 0))
            {
                self.error = Some("smoke test lacked display/capture/audio output".into());
            }
            println!(
                "smoke finished: {} presented frames, VM tick {}, pending messages {}",
                self.presented,
                self.game.as_ref().map_or(0, |g| g.vm.state().tick),
                self.game
                    .as_ref()
                    .map_or(0, |g| g.pending_messages().count())
            );
            self.stopped = true;
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        self.max_tick_lag = self
            .max_tick_lag
            .max(now.saturating_duration_since(clock.next_tick));
        let mut advanced = false;
        // Catch short OS/render jitter without slowing script time. Input is
        // sampled on each update; at most four updates run before pumping events.
        for _ in 0..playback_clock::MAX_UPDATES_PER_PUMP {
            if !self.clock.as_mut().unwrap().begin_tick(Instant::now()) { break; }
            if self.error.is_none() {
                if let Some(game) = &mut self.game {
                    let update_started = Instant::now();
                    let mut sample = self
                        .keyboard
                        .sample_with_bindings(game.vm.input_policy(), game.vm.input_bindings());
                    if let Some(press) = self.startup_confirm_tick {
                        // Explicit acceptance input through the ordinary physical-key adapter.
                        // The default map, Common22 and title scripts all remain active.
                        let mut keyboard = keyboard::Keyboard::default();
                        keyboard.key(
                            winit::keyboard::KeyCode::Enter,
                            (press..press + 3).contains(&game.vm.state().tick),
                        );
                        sample = keyboard.sample_with_pad_buttons(
                            game.vm.input_policy(),
                            game.vm.input_bindings(),
                            0,
                        );
                        if game.vm.state().tick == press {
                            println!("STARTUP ACCEPTANCE physical Enter at VM tick {press}; external input isolated");
                        }
                    }
                    if let Some(press) = self.probe_press_tick {
                        // Explicit test injection, never active during normal/diagnostic game entry.
                        sample = wolf_vm::input::KeyboardSample::default();
                        sample.set(30, game.vm.state().tick + 1 == press);
                    }
                    game.vm.set_keyboard(sample);
                    if let Some(input) = self.input_received.take() {
                        self.metrics.record("input_event_to_sample", input.elapsed());
                        self.sampled_input.get_or_insert(input);
                    }
                    if let Err(e) = game.tick() {
                        eprintln!("{e}");
                        self.error = Some(e.clone());
                        self.messages.push(e);
                        if self.messages.len() > 4 {
                            self.messages.remove(0);
                        }
                    }
                    if game.exit_requested() {
                        self.stopped = true;
                        event_loop.exit();
                        return;
                    }
                    self.tick_count += 1;
                    if self.performance_report.is_some() { self.metrics.record("logic_update", update_started.elapsed()); }
                    let state = game.vm.state();
                    if self.trace_vm {
                    let status = state
                        .fibers
                        .values()
                        .map(|f| {
                            format!(
                                "fiber {} {:?} pc={:?}",
                                f.id,
                                f.wait,
                                f.frames.last().map(|frame| frame.pc)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    let status = if self.input_probe {
                        format!(
                            "input probe: {status}; cself58={:?}",
                            state.world.commons.numbers.get(&2658)
                        )
                    } else {
                        status
                    };
                    if status != self.last_vm_status {
                        println!("VM tick {}: {status}", state.tick);
                        self.last_vm_status = status;
                    }
                    }
                    if let (Some(window), Some(title)) =
                        (&self.window, state.world.systems.strings.get(&60))
                    {
                        if self.applied_title.as_ref() != Some(title) {
                            window.set_title(title);
                            self.applied_title = Some(title.clone());
                        }
                    }
                    if let Some((token, text)) = game.pending_messages().next() {
                        let notice =
                            format!("message {}:{}: {text}", token.session, token.sequence);
                        if self.announced_message.as_deref() != Some(&notice) {
                            println!("VM message waiting (Space acknowledges): {notice}");
                            self.announced_message = Some(notice);
                        }
                    }
                }
            }
            advanced = true;
        }
        let clock = self.clock.as_ref().unwrap();
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            clock
                .deadline
                .map_or(clock.next_tick, |end| end.min(clock.next_tick)),
        ));
        if advanced {
            if let Err(error) = self.apply_window_mode() {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
            // User-selected host policy: always present at 60 Hz. The script's
            // frame-skip setting remains readable but cannot lower output FPS.
            if let Some(window) = &self.window { window.request_redraw(); }
        }
    }
}
struct Options {
    restore_checkpoint: Option<PathBuf>,
    performance_report: Option<PathBuf>,
    trace_vm: bool,
    root: Option<PathBuf>,
    save_root: Option<PathBuf>,
    audio: Option<PathBuf>,
    capture: Option<PathBuf>,
    smoke: Duration,
    program: Option<String>,
    input_probe: bool,
    media_probe: bool,
    capture_tick: u64,
    probe_press_tick: Option<u64>,
    startup_confirm_tick: Option<u64>,
}
fn parse_args() -> Result<Options, String> {
    let mut restore_checkpoint = None;
    let mut performance_report = None;
    let mut trace_vm = false;
    let mut args = std::env::args_os().skip(1);
    let mut root = None;
    let mut save_root = None;
    let mut audio = None;
    let mut smoke = Duration::ZERO;
    let mut capture = None;
    let mut program = None;
    let mut input_probe = false;
    let mut media_probe = false;
    let mut capture_tick = 0;
    let mut probe_press_tick = None;
    let mut startup_confirm_tick = None;
    while let Some(arg) = args.next() {
        let value = |args: &mut std::iter::Skip<std::env::ArgsOs>, option: &str| {
            args.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{option} requires a value"))
        };
        match arg.to_string_lossy().as_ref() {
            "--root" => root = Some(value(&mut args, "--root")?),
            "--save-root" => save_root = Some(value(&mut args, "--save-root")?),
            "--restore-checkpoint" => restore_checkpoint = Some(value(&mut args, "--restore-checkpoint")?),
            "--audio" => audio = Some(value(&mut args, "--audio")?),
            "--capture" => capture = Some(value(&mut args, "--capture")?),
            "--performance-report" => performance_report = Some(value(&mut args, "--performance-report")?),
            "--trace-vm" => trace_vm = true,
            "--program" => {
                program = Some(
                    value(&mut args, "--program")?
                        .to_string_lossy()
                        .into_owned(),
                )
            }
            "--input-probe" => input_probe = true,
            "--media-probe" => media_probe = true,
            "--capture-tick" => {
                capture_tick = value(&mut args, "--capture-tick")?
                    .to_string_lossy()
                    .parse::<u64>()
                    .map_err(|_| "invalid capture tick")?;
            }
            "--startup-confirm-tick" => {
                let tick = value(&mut args, "--startup-confirm-tick")?
                    .to_string_lossy()
                    .parse::<u64>()
                    .map_err(|_| "invalid startup confirmation tick")?;
                if !(2..=216000).contains(&tick) {
                    return Err("startup confirmation tick must be 2..216000".into());
                }
                startup_confirm_tick = Some(tick);
            }
            "--probe-press-tick" => {
                let tick = value(&mut args, "--probe-press-tick")?
                    .to_string_lossy()
                    .parse::<u64>()
                    .map_err(|_| "--probe-press-tick must be from 2 to 216000".to_owned())?;
                if !(2..=216000).contains(&tick) {
                    return Err("--probe-press-tick must be from 2 to 216000".into());
                }
                probe_press_tick = Some(tick);
            }
            "--smoke-seconds" => {
                let value = value(&mut args, "--smoke-seconds")?;
                let seconds = value
                    .to_string_lossy()
                    .parse::<u64>()
                    .map_err(|_| "--smoke-seconds must be an integer from 1 to 3600".to_owned())?;
                if !(1..=3600).contains(&seconds) {
                    return Err("--smoke-seconds must be from 1 to 3600".into());
                }
                smoke = Duration::from_secs(seconds);
            }
            "--help" => {
                return Err("wolf-player [--root GAME_DIR] [--save-root SAVE_DIR] [--program EVENT_ID | --input-probe | --media-probe] [--restore-checkpoint PATH (smoke only; requires --save-root)] [--capture-tick N] [--startup-confirm-tick N (acceptance only)] [--probe-press-tick N (probe only)] [--audio ...] [--capture screenshot.png] [--smoke-seconds 1..3600]".into());
            }
            other => {
                eprintln!("unknown option: {other}");
                std::process::exit(2);
            }
        }
    }
    if (input_probe as u8 + media_probe as u8 + program.is_some() as u8) > 1 {
        return Err("--program and --input-probe are mutually exclusive".into());
    }
    if probe_press_tick.is_some() && (!input_probe || smoke.is_zero()) {
        return Err("--probe-press-tick requires --input-probe and --smoke-seconds".into());
    }
    if startup_confirm_tick.is_some()
        && (input_probe || media_probe || program.is_some() || smoke.is_zero())
    {
        return Err("--startup-confirm-tick requires default startup and --smoke-seconds".into());
    }
    if restore_checkpoint.is_some()
        && (smoke.is_zero() || save_root.is_none() || input_probe || media_probe
            || program.is_some() || startup_confirm_tick.is_some() || probe_press_tick.is_some())
    {
        return Err("--restore-checkpoint requires --smoke-seconds and --save-root, and cannot combine with alternate entry or injected startup/probe input".into());
    }
    Ok(Options {
        restore_checkpoint,
        performance_report,
        trace_vm,
        root,
        save_root,
        audio,
        capture,
        smoke,
        program,
        input_probe,
        media_probe,
        capture_tick,
        probe_press_tick,
        startup_confirm_tick,
    })
}
fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) if message.starts_with("wolf-player ") => {
            println!("{message}");
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let root = options.root.clone().unwrap_or(exe);
    let mut app = match App::load(root, &options) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("preview setup failed: {e}");
            std::process::exit(1)
        }
    };
    let event_loop = EventLoop::new().unwrap_or_else(|e| {
        eprintln!("event loop: {e}");
        std::process::exit(1)
    });
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop: {e}");
        std::process::exit(1)
    }
    if let Some(path) = &app.performance_report {
        let report = serde_json::json!({
            "target_hz": 60,
            "presented_frames": app.presented,
            "vm_tick": app.game.as_ref().map(|g|g.vm.state().tick),
            "long_stalls": app.clock.as_ref().map(|c|c.stalls),
            "timings": app.metrics.report(),
            "error": app.error,
            "input_latency_scope": "OS event receipt to sample/present submission; excludes device and scanout latency"
        });
        if let Err(error) = serde_json::to_vec_pretty(&report).map_err(|e|e.to_string()).and_then(|bytes|std::fs::write(path,bytes).map_err(|e|e.to_string())) {
            eprintln!("performance report: {error}");
            std::process::exit(1);
        }
    }
    if let Some(e) = app.error {
        eprintln!("preview failed: {e}");
        std::process::exit(1)
    }
}
