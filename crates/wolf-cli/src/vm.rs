use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};
use wolf_assets::{
    wolf350::{Archive, Limits as ArchiveLimits},
    Asset, LooseMount,
};
use wolf_format::{Limits, RawCommand};
use wolf_ir::{Coverage, Library, Program, Source, Target};
enum Input {
    Loose(LooseMount),
    Archives(BTreeMap<String, Archive>),
}
impl Input {
    fn read(&self, key: &str) -> Result<Asset, Box<dyn Error>> {
        match self {
            Self::Loose(m) => Ok(m.read(key, wolf_format::PROFILE)?),
            Self::Archives(a) => {
                let (archive, member) = key.split_once('/').ok_or("missing archive name")?;
                let mut asset = a.get(archive).ok_or("archive missing")?.read(member)?;
                asset.id.original_path = key.into();
                asset.id.key = key.into();
                Ok(asset)
            }
        }
    }
    fn maps(&self) -> Result<Vec<String>, Box<dyn Error>> {
        Ok(match self {
            Self::Loose(m) => m.list("MapData")?,
            Self::Archives(a) => a["MapData"]
                .entries()
                .map(|e| format!("MapData/{}", e.path))
                .collect(),
        })
    }
}
pub(crate) fn archive(path: &Path) -> Result<Archive, Box<dyn Error>> {
    let limits = ArchiveLimits::default();
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limits.archive_bytes as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(Archive::open(bytes, limits)?)
}
pub fn run(args: Vec<OsString>) -> Result<(), Box<dyn Error>> {
    let mut args = args.into_iter();
    let mut input = None;
    let mut execute = None;
    let mut ticks = 60u32;
    let mut host_enabled = false;
    let mut host_actions_path = None;
    while let Some(a) = args.next() {
        if (a == "--assets" || a == "--archives") && input.is_none() {
            let root = PathBuf::from(args.next().ok_or("missing asset root")?);
            input = Some(if a == "--assets" {
                Input::Loose(LooseMount::new(
                    root,
                    "loose-v1",
                    Limits::default().max_stored_bytes,
                )?)
            } else {
                Input::Archives(BTreeMap::from([
                    ("BasicData".into(), archive(&root.join("BasicData.wolf"))?),
                    ("MapData".into(), archive(&root.join("MapData.wolf"))?),
                ]))
            });
        } else if a == "--run" && execute.is_none() {
            execute = Some(
                args.next()
                    .ok_or("missing program ID")?
                    .into_string()
                    .map_err(|_| "non-UTF8 program")?,
            );
        } else if a == "--host" && !host_enabled {
            host_enabled = true;
        } else if a == "--host-actions" && host_actions_path.is_none() {
            host_actions_path = Some(PathBuf::from(args.next().ok_or("missing host action log")?));
        } else if a == "--ticks" {
            ticks = args
                .next()
                .ok_or("missing tick count")?
                .to_str()
                .ok_or("invalid ticks")?
                .parse()?;
            if ticks > 100_000 {
                return Err("tick limit 100000".into());
            }
        } else {
            return Err(format!("unknown/repeated argument {a:?}").into());
        }
    }
    let input = input.ok_or("--assets or --archives is required")?;
    if (host_enabled || host_actions_path.is_some()) && execute.is_none() {
        return Err("host options require --run".into());
    }
    let mut host_actions: BTreeMap<u64, Vec<wolf_vm::host::Action>> = BTreeMap::new();
    if let Some(path) = host_actions_path {
        host_enabled = true;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((1 << 20) + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1 << 20 {
            return Err("host action log exceeds 1 MiB".into());
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Actions {
            tick: u64,
            actions: Vec<wolf_vm::host::Action>,
        }
        let frames: Vec<Actions> = serde_json::from_slice(&bytes)?;
        for frame in frames {
            if frame.tick == 0
                || frame.tick > u64::from(ticks)
                || frame.actions.len() > 16_384
                || host_actions.insert(frame.tick, frame.actions).is_some()
            {
                return Err("invalid/duplicate host action tick or action limit".into());
            }
        }
    }
    let mut programs: Vec<Program> = Vec::new();
    let mut coverage = Coverage::default();
    let mut failures = Vec::new();
    let mut add = |asset: &Asset, event, page, name: String, commands: &[RawCommand]| {
        let id = match page {
            Some(page) => format!("{}/{event}/{page}", asset.id.key),
            None => format!("common/{event}"),
        };
        let source = Source {
            asset: asset.id.key.clone(),
            content_blake3: asset.id.content_blake3.clone(),
            event,
            page,
            command: 0,
            span: [0, 0],
        };
        match wolf_ir::compile(id.clone(), name.clone(), source.clone(), commands) {
            Ok(p) => {
                coverage.add(&p);
                programs.push(p);
            }
            Err(e) => {
                coverage.failed_commands += commands.len();
                failures.push(
                    serde_json::json!({"program": id, "error": e, "commands": commands.len()}),
                );
                let fallback = commands
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let mut source = source.clone();
                        source.command = i;
                        source.span = [c.span.start, c.span.end];
                        wolf_ir::Instruction {
                            source,
                            raw: Some(c.clone()),
                            op: wolf_ir::Op::Unsupported {
                                opcode: c.opcode,
                                reason: "program control flow failed structural validation".into(),
                            },
                        }
                    })
                    .collect();
                let p = Program {
                    id,
                    name,
                    profile: wolf_ir::PROFILE.into(),
                    code: fallback,
                };
                coverage.add(&p);
                programs.push(p);
            }
        }
    };
    let limits = Limits::default();
    for key in input.maps()? {
        if !key.ends_with(".mps") {
            continue;
        }
        let asset = input.read(&key)?;
        let parsed = wolf_format::parse_map(&asset.bytes, &limits)?;
        for e in &parsed.value.events {
            for (page, p) in e.pages.iter().enumerate() {
                add(&asset, e.id, Some(page as u32), String::new(), &p.commands);
            }
        }
    }
    let asset = input.read("BasicData/CommonEvent.dat")?;
    const PREFIX: &[u8] = b"Extracting data from encrypted files violates the guidelines.\0";
    let bytes = asset.bytes.strip_prefix(PREFIX).unwrap_or(&asset.bytes);
    let common = wolf_format::parse_common(bytes, &limits)?;
    for e in &common.value.events {
        add(&asset, e.id, None, e.name.text()?.to_owned(), &e.commands);
    }
    let count = programs.len();
    let library = Library::new(programs)?;
    let identity = library.identity().to_string();
    let mut execution = None;
    if let Some(id) = execute {
        let mut vm = wolf_vm::runtime::Vm::new(library, wolf_vm::runtime::Config::default(), 1, 1)?;
        vm.spawn(Target::Id(id), 1, vec![])?;
        let mut trace = Vec::new();
        let mut host = wolf_vm::host::Host::new(4096, 16_384)?;
        for _ in 0..ticks {
            let poll = if host_enabled {
                let actions = host_actions
                    .remove(&(vm.state().tick + 1))
                    .unwrap_or_default();
                host.poll(&vm, &actions, |path| {
                    input.read(path).map(|_| ()).map_err(|e| e.to_string())
                })?
            } else {
                wolf_vm::host::Poll::default()
            };
            let completions = poll.completions.clone();
            let report = vm.tick(poll.completions)?;
            trace.push(
                serde_json::json!({"report": report, "state_hash": vm.state_hash()?,
                "host_completions": completions, "rejected_host_actions": poll.rejected_actions}),
            );
            if vm.state().fibers.values().all(|f| {
                matches!(
                    f.wait,
                    wolf_vm::runtime::Wait::Done | wolf_vm::runtime::Wait::Fault
                ) || (!host_enabled && matches!(f.wait, wolf_vm::runtime::Wait::Service(_)))
            }) {
                break;
            }
        }
        execution = Some(serde_json::json!({"trace": trace, "state": vm.state()}));
    }
    println!(
        "{}",
        serde_json::to_string(
            &serde_json::json!({"profile": wolf_ir::PROFILE, "programs": count,
        "library_blake3": identity, "coverage": coverage, "structural_failures": failures, "execution": execution,
        "qualification": "lowered includes native_host routing; host parameter forms may still fault at execution. Counts are compiler accounting, not complete opcode support or original-engine execution parity"})
        )?
    );
    Ok(())
}
