use serde_json::json;
use std::{error::Error, ffi::OsString, path::PathBuf};
use wolf_format::Limits;

pub fn run(args: Vec<OsString>) -> Result<(), Box<dyn Error>> {
    if args.len() != 2 || args[0] != "--archives" {
        return Err("usage: wolf-cli inspect-startup --archives <Data-directory>".into());
    }
    let root = PathBuf::from(&args[1]);
    let basic = crate::vm::archive(&root.join("BasicData.wolf"))?;
    let maps = crate::vm::archive(&root.join("MapData.wolf"))?;
    let game_asset = basic.read("Game.dat")?;
    let config = wolf_format::parse_game(&game_asset.bytes, &Limits::default())?;
    let asset = basic.read("SysDatabase.dat")?;
    let database = wolf_format::parse_database(&asset.bytes, &Limits::default())?;
    let initial = wolf_ir::startup::initial_position(&database.value)?;
    let mut catalog = Vec::new();
    for id in 0..database.value.types[0].rows.len() {
        let path = wolf_ir::startup::map_path(&database.value, id as u32)?;
        let asset = maps.read(path.strip_prefix("MapData/").unwrap())?;
        let parsed = wolf_format::parse_map(&asset.bytes, &Limits::default())?;
        catalog.push(json!({"id": id, "path": path, "identity": asset.id,
            "events": parsed.value.events.len(), "version": parsed.value.version}));
    }
    let report = json!({
        "schema": "wolf-startup-inspection-v1",
        "profile": wolf_ir::startup::PROFILE,
        "database_identity": asset.id,
        "database_types": database.value.types.len(),
        "initial_position": initial,
        "map_catalog": catalog,
        "game_title_valid_utf8": config.title.text().is_ok(),
        "execution_status": "blocked: event-page eligibility and startup scheduling are not implemented"
    });
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
    Ok(())
}
