//! Native242: A1F9B0 rectangle write, A1F8C0/A1F900 autotile seams.
use super::Scene;
use wolf_vm::runtime::NativeContext;

impl Scene {
    pub(super) fn replace_tiles_command(&mut self, context: &mut NativeContext<'_>) -> Result<bool, String> {
        let raw = context.instruction.raw.as_ref().ok_or("missing tile command")?;
        if raw.args.len() != 6 || !raw.strings.is_empty() || raw.movement.is_some() {
            return Err("242 tile replacement requires six operands and no strings/route".into());
        }
        let mut values = [0; 6];
        for (value, operand) in values.iter_mut().zip(&raw.args) { *value = context.number(*operand)?; }
        // Literal editor numbers and values read from variables use different
        // converters (A1FA50..76). Do not infer this from the resolved value.
        values[5] = if raw.args[5] < 1_000_000 {
            literal_tile_number(values[5])?
        } else {
            let version = self.native_version.ok_or("tile replacement needs Game.dat version")?;
            if version < 230 && values[5] >= 16 { values[5].wrapping_add(16) } else { values[5] }
        };
        self.replace_tiles(values)?;
        Ok(true)
    }

    fn replace_tiles(&mut self, [layer, x, y, width, height, number]: [i32; 6]) -> Result<(), String> {
        let layer = i64::from(layer) - 1;
        let properties = self.passability.as_ref().ok_or("tile replacement needs tileset properties")?;
        // A1FA76..DA: invalid selectors are accepted without modifying the map.
        if layer < 0 || layer >= i64::from(self.layers) || number < 0 || number as usize >= properties.len()
            || x < 0 || x >= self.width || y < 0 || y >= self.height { return Ok(()); }
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let end_x = x.saturating_add(width);
        let end_y = y.saturating_add(height);
        let map_width = self.width as usize;
        let map_height = self.height as usize;
        let index = |x: i32, y: i32| (layer as usize * map_width + x as usize) * map_height + y as usize;
        let tiles = self.tiles.as_mut().ok_or("tile replacement on sentinel map has no tile buffer")?;
        let required = self.layers as usize * map_width * map_height;
        if tiles.len() != required { return Err("tile replacement map buffer mismatch".into()); }
        let word = if number < 32 { (number + 1) * 100_000 + 4444 } else { number - 32 };
        for px in x..end_x { for py in y..end_y { tiles[index(px, py)] = word; } }
        // Recompute the replacement boundary and one adjacent cell. Interior
        // cells already have four connected quarters. Out-of-map neighbors
        // count as connected, independently of map loop settings.
        for px in (x - 1).max(0)..end_x.saturating_add(1).min(self.width) {
            for py in (y - 1).max(0)..end_y.saturating_add(1).min(self.height) {
                if px > x && px < end_x - 1 && py > y && py < end_y - 1 { continue; }
                let group = tiles[index(px, py)] / 100_000;
                if group < 2 { continue; }
                let mut adjacent = [true; 9];
                for dy in -1..=1 { for dx in -1..=1 {
                    let nx = px + dx; let ny = py + dy;
                    if (0..self.width).contains(&nx) && (0..self.height).contains(&ny) {
                        adjacent[((dy + 1) * 3 + dx + 1) as usize] = tiles[index(nx, ny)] / 100_000 == group;
                    }
                } }
                tiles[index(px, py)] = group * 100_000 + autotile_shape(adjacent);
            }
        }
        Ok(())
    }
}

fn literal_tile_number(number: i32) -> Result<i32, String> {
    // AF3D31..AF3EA0. Negative editor identifiers encode extra autotile slots.
    match number {
        8.. => Ok(number.wrapping_add(24)),
        -34..=-19 | -18..=-11 => Ok(-3 - number),
        0..=7 => Ok(number),
        _ => Err(format!("invalid literal tile identifier {number}")),
    }
}

fn autotile_shape(n: [bool; 9]) -> i32 {
    fn quarter(vertical: bool, horizontal: bool, diagonal: bool) -> i32 {
        match (vertical, horizontal, diagonal) {
            (true, true, true) => 4, (true, true, false) => 3,
            (true, false, _) => 1, (false, true, _) => 2, (false, false, _) => 0,
        }
    }
    quarter(n[1], n[3], n[0]) * 1000 + quarter(n[1], n[5], n[2]) * 100
        + quarter(n[7], n[3], n[6]) * 10 + quarter(n[7], n[5], n[8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wolf_format::{Map, RawCommand, Span, WolfString};

    fn scene() -> Scene {
        let span = Span { start: 0, end: 0 };
        let map = Map { version: 103, unknown_byte_24: 0, label: WolfString { raw: vec![0], span },
            tileset: 0, width: 4, height: 3, unknown: None, layers: 2,
            tiles: Some(vec![100_000; 24]), tiles_span: span, events: vec![] };
        let mut scene = Scene::initial(&map, &wolf_ir::startup::InitialPosition {
            map_id: 9, x: 0, y: 0, map_path: "tile-test".into() }, vec![0; 1024]).unwrap();
        scene.native_version = Some(371);
        scene.collision_layer_limit = 2;
        scene.set_passability(vec![0; 1024], 0).unwrap();
        scene
    }

    #[test]
    fn rectangle_updates_selected_layer_clips_and_survives_snapshot() {
        let mut scene = scene();
        scene.tags[148] = 7;
        scene.passability.as_mut().unwrap()[148] = 15;
        scene.replace_tiles([2, 2, 1, 99, 99, 148]).unwrap();
        assert!(scene.tiles().unwrap()[..12].iter().all(|n| *n == 100_000));
        assert_eq!(scene.tiles().unwrap()[19], 116);
        assert_eq!(scene.top_tile_number(4, 2).unwrap(), 148);
        assert_eq!(scene.tile_tag(4, 2).unwrap(), 7);
        assert!(!scene.passable_query(4, 2, false).unwrap());
        let restored: Scene = serde_json::from_slice(&serde_json::to_vec(&scene).unwrap()).unwrap();
        assert_eq!(restored.tiles(), scene.tiles());
        let before = scene.tiles().unwrap().to_vec();
        for args in [[0,0,0,1,1,2], [3,0,0,1,1,2], [1,-1,0,1,1,2], [1,0,3,1,1,2], [1,0,0,1,1,1024]] {
            scene.replace_tiles(args).unwrap();
            assert_eq!(scene.tiles().unwrap(), before);
        }
    }

    #[test]
    fn autotile_seams_recompute_neighbors_and_map_edges() {
        let mut scene = scene();
        scene.replace_tiles([1,1,1,1,1,1]).unwrap();
        assert_eq!(scene.tiles().unwrap()[4], 200_000);
        scene.replace_tiles([1,2,1,1,1,1]).unwrap();
        assert_eq!(scene.tiles().unwrap()[4], 200_202);
        assert_eq!(scene.tiles().unwrap()[7], 202_020);
        scene.replace_tiles([1,0,0,4,3,1]).unwrap();
        assert!(scene.tiles().unwrap()[..12].iter().all(|word| *word == 204_444));
        scene.replace_tiles([1,1,1,1,1,32]).unwrap();
        assert_eq!(scene.tiles().unwrap()[4], 0);
        assert_eq!(scene.tiles().unwrap()[0], 204_443);
        assert_eq!(scene.tiles().unwrap()[3], 204_411);
    }

    #[test]
    fn tile_command_resolves_all_operands_and_distinguishes_literal_numbers() {
        let mut scene = scene();
        let span = Span { start: 0, end: 1 };
        let commands: Vec<_> = [[1,0,0,1,1,148], [1_100_001,1_100_002,1_100_003,1_100_004,1_100_005,1_100_006]]
            .into_iter().map(|args| RawCommand { span, opcode: 242, args: args.to_vec(), indent: 0,
                strings: vec![], terminator: 0, movement: None, extension_length: 0, extension: vec![] }).collect();
        let program = wolf_ir::compile("tile".into(), "tile".into(), wolf_ir::Source {
            asset: "fixture".into(), content_blake3: "fixture".into(), event: 0, page: Some(0), command: 0, span: [0,1] }, &commands).unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(wolf_ir::Library::new(vec![program]).unwrap(), Default::default(), 1, 1).unwrap();
        vm.spawn(wolf_ir::Target::Id("tile".into()), 0, vec![]).unwrap();
        let report = vm.tick_with_host(vec![], &mut |ctx| {
            for (index, value) in [2,1,0,1,1,148].into_iter().enumerate() { ctx.set_number(1_100_001 + index as i32, value)?; }
            scene.execute(ctx)
        }).unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(scene.top_tile_number(0,0).unwrap(), 172);
        assert_eq!(scene.top_tile_number(2,0).unwrap(), 148);
        for (input, output) in [(0,0),(7,7),(8,32),(148,172),(-11,8),(-18,15),(-19,16),(-34,31)] {
            assert_eq!(literal_tile_number(input).unwrap(), output);
        }
        assert!(literal_tile_number(-1).is_err());
    }
}
