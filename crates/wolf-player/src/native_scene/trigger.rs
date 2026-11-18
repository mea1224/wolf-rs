//! GamePro 9F63E0 (overlap), 9F6900 (confirm), 9F72D0 (blocked
//! player movement), 9F94A0 (event touch), and 9FF930 (counter tiles).
//! Coordinates and page expansion bytes are native half-tile units.
use super::{Actor, Scene};

fn delta(direction: i32, width: i32) -> [i32; 2] {
    [match direction { 1 | 4 | 7 => -width, 3 | 6 | 9 => width, _ => 0 },
     match direction { 1..=3 => width, 7..=9 => -width, _ => 0 }]
}

fn expansion(page: &wolf_format::Page) -> [i32; 2] {
    if page.shadow_collision.len() < 3 { [0, 0] }
    else { [i32::from(page.shadow_collision[1]), i32::from(page.shadow_collision[2])] }
}

impl Scene {
    pub fn player_control_ready(&self) -> bool { self.player_route.player_control_ready() }

    pub(super) fn player_direction_clear(&self, target: [i32; 2], direction: i32) -> Result<bool, String> {
        if self.player_route.through { return Ok(true); }
        let step = self.player_route.step_width();
        let half = self.player_route.half_height();
        let axes: &[i32] = match direction { 1 => &[2, 4], 3 => &[2, 6], 7 => &[8, 4], 9 => &[8, 6], _ => std::slice::from_ref(&direction) };
        for &axis in axes {
            if !self.passable_direction(self.player_x2, self.player_y2, half, axis, step)?
                || !self.passable_direction(target[0], target[1], half, 10 - axis, step)? { return Ok(false); }
        }
        Ok(true)
    }

    pub(super) fn player_destination_clear(&self, target: [i32; 2]) -> Result<bool, String> {
        let half = self.player_route.half_height();
        if self.loop_mode & 1 == 0 && !(0..self.width * 2 - 1).contains(&target[0])
            || self.loop_mode & 2 == 0 && !(-i32::from(half)..self.height * 2 - 1).contains(&target[1]) {
            return Ok(false);
        }
        if self.player_route.through { return Ok(true); }
        if !self.passable_query(target[0], target[1], half)? { return Ok(false); }
        if half && self.player_route.step_width() == 1 && !self.passable_query(target[0], target[1] - 1, half)? { return Ok(false); }
        for actor in self.events.values() {
            if !actor.active_page || actor.route.through || actor.route.jump_active() { continue; }
            let graphic = actor.route.graphic.as_deref().unwrap_or(&actor.graphic);
            if actor.tile_id == -1 && graphic.strip_suffix(&[0]).unwrap_or(graphic).is_empty() { continue; }
            // 9EFD50..9EFE6A tests a two-half-unit horizontal footprint and
            // the actor's half-height; page interaction expansion is separate.
            if self.trigger_images([actor.x2, actor.y2]).iter().any(|p| {
                (target[0] - 1..=target[0] + 1).contains(&p[0])
                    && (p[1] == target[1] || actor.route.half_height() && p[1] == target[1] + 1
                        || half && p[1] == target[1] - 1)
            }) { return Ok(false); }
        }
        Ok(true)
    }

    /// 9F8A20: invisible or through events on a passable floor are reached by
    /// overlap; they do not occupy a solid front-facing interaction footprint.
    fn solid_trigger(&self, actor: &Actor) -> Result<bool, String> {
        let graphic = actor.route.graphic.as_deref().unwrap_or(&actor.graphic);
        let graphic = graphic.strip_suffix(&[0]).unwrap_or(graphic);
        if (actor.tile_id != -1 || !graphic.is_empty()) && !actor.route.through { return Ok(true); }
        // 9F8A20 uses the global step mode, with half-height disabled.
        Ok(!self.passable_direction(actor.x2, actor.y2, false, 0, 1 + i32::from(self.configured_step_mode == 1))?)
    }

    fn trigger_images(&self, position: [i32; 2]) -> Vec<[i32; 2]> {
        let mut result = vec![position];
        if self.loop_mode & 1 != 0 {
            result.push([position[0] - self.width * 2, position[1]]);
            result.push([position[0] + self.width * 2, position[1]]);
        }
        if self.loop_mode & 2 != 0 {
            let x_images = result.clone();
            for p in x_images {
                result.push([p[0], p[1] - self.height * 2]);
                result.push([p[0], p[1] + self.height * 2]);
            }
        }
        result
    }

    fn overlaps_trigger(&self, actor: &Actor, page: &wolf_format::Page, target: [i32; 2], front: bool, direction: i32) -> bool {
        let [ex, ey] = expansion(page);
        let event_half = i32::from(actor.route.half_height());
        let player_half = i32::from(self.player_route.half_height());
        let offsets = self.player_route.pixel_offsets();
        let horizontal = matches!(direction, 4 | 6);
        let y_player = if !front || horizontal { player_half } else { 0 };
        let y_extra = if front && horizontal { i32::from(offsets[1] >= 1) } else { 0 };
        let x_extra = if front || page.conditions.first() == Some(&4) { 1 } else { 0 };
        let x_offset = if front && matches!(direction, 2 | 8) { i32::from(offsets[0] >= 1) } else { 0 };
        self.trigger_images([actor.x2, actor.y2]).iter().any(|p| {
            p[0] >= target[0] - ex - x_extra && p[0] <= target[0] + ex + x_extra + x_offset
                && p[1] >= target[1] - ey - y_player
                && p[1] <= target[1] + ey + event_half + y_extra
        })
    }

    fn front_target(&self, direction: i32) -> [i32; 2] {
        let mut d = delta(direction, self.player_route.step_width());
        if direction >= 7 && self.player_route.half_height() { d[1] = -2; }
        let offset = self.player_route.pixel_offsets();
        [self.player_x2 + d[0] + i32::from(matches!(direction, 1 | 4 | 7) && offset[0] >= 1),
         self.player_y2 + d[1] + i32::from(direction >= 7 && offset[1] >= 1)]
    }

    fn counter_at(&self, target: [i32; 2]) -> Result<bool, String> {
        if self.tiles.is_none() { return Ok(false); }
        let flags = self.passability.as_ref().ok_or("counter query requires tileset properties")?;
        // FA5238 nonzero uses a single tile; zero samples six half-grid
        // positions (x, x-1, x+1) at y and y+1, as 9FFA87..9FFFDA.
        let samples: &[[i32; 2]] = if self.configured_step_mode != 0 { &[[0, 0]] }
            else { &[[0, 0], [-1, 0], [1, 0], [0, 1], [-1, 1], [1, 1]] };
        for layer in (0..self.layers as usize).rev() {
            let indices = samples.iter().map(|d| self.tile_index(layer, target[0] + d[0], target[1] + d[1]))
                .collect::<Result<Vec<_>, _>>()?;
            if indices.iter().any(|&index| index < 0) { continue; }
            if indices.iter().any(|&index| flags.get(index as usize).is_some_and(|flag| flag & 0x80 != 0)) { return Ok(true); }
        }
        Ok(false)
    }

    pub fn map_trigger_candidates(&self, pages: &[(u32, &wolf_format::Page)], confirm: bool, landed: bool, blocked_direction: Option<i32>) -> Result<Vec<u32>, String> {
        let mut result = Vec::new();
        if self.player_route.jump_active() { return Ok(result); }
        let player = [self.player_x2, self.player_y2];
        for &(id, page) in pages {
            let Some(actor) = self.events.get(&(id as i32)) else { continue; };
            if !actor.active_page || actor.route.jump_active() { continue; }
            let trigger = page.conditions.first().copied().unwrap_or(255);
            let overlap = trigger == 4 || trigger == 3 && landed || trigger == 0 && confirm;
            if overlap && self.overlaps_trigger(actor, page, player, false, 0) { result.push(id); }
            if let Some(direction) = blocked_direction {
                if matches!(trigger, 3 | 4) && self.solid_trigger(actor)?
                    && self.overlaps_trigger(actor, page, self.front_target(direction), true, direction) {
                    result.push(id);
                }
            }
        }
        if confirm {
            let direction = i32::from(self.direction);
            let mut target = self.front_target(direction);
            for counter_pass in 0..2 {
                let mut found = false;
                for &(id, page) in pages {
                    if page.conditions.first() != Some(&0) { continue; }
                    let Some(actor) = self.events.get(&(id as i32)) else { continue; };
                    if actor.active_page && !actor.route.jump_active() && self.solid_trigger(actor)?
                        && self.overlaps_trigger(actor, page, target, true, direction) {
                        result.push(id);
                        found = true;
                        break;
                    }
                }
                if found || counter_pass != 0 || !self.counter_at(target)? { break; }
                let d = delta(direction, 2);
                target = [target[0] + d[0], target[1] + d[1]];
            }
        }
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wolf_format::{Map, MapEvent, Page, Span, WolfString};

    fn fixture(trigger: u8, position: [u32; 2], expansion: [u8; 2], visible: bool) -> (Scene, Page) {
        let span = Span { start: 0, end: 0 };
        let empty = WolfString { raw: vec![0], span };
        let page = Page {
            span, unknown: u32::MAX, graphic: if visible { WolfString { raw: b"actor.png\0".to_vec(), span } } else { empty.clone() },
            graphic_fields: vec![2, 1, 255, 0], conditions: vec![trigger], movement: vec![0, 2, 3, 6],
            flags: 0, route_flags: 0, routes: vec![], commands: vec![], features: 0,
            shadow_collision: vec![0, expansion[0], expansion[1]], page_transfer: None,
        };
        let map = Map { version: 103, unknown_byte_24: 0, label: empty.clone(), tileset: 0,
            width: 10, height: 10, unknown: None, layers: 1, tiles: Some(vec![0; 100]), tiles_span: span,
            events: vec![MapEvent { span, id: 7, name: empty, x: position[0], y: position[1], pages: vec![page.clone()] }] };
        let mut scene = Scene::initial(&map, &wolf_ir::startup::InitialPosition { map_id: 1, x: 3, y: 3, map_path: "fixture".into() }, vec![]).unwrap();
        scene.tile_pixels = Some(32);
        scene.collision_layer_limit = 1;
        scene.set_passability(vec![0; 33], 0).unwrap();
        scene.set_initial_page(7, Some(&page)).unwrap();
        (scene, page)
    }

    #[test]
    fn invisible_expanded_action_uses_overlap_and_does_not_block_walk() {
        let (scene, page) = fixture(0, [3, 4], [3, 4], false);
        assert!(scene.map_trigger_candidates(&[(7, &page)], false, false, None).unwrap().is_empty());
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap(), [7]);
        assert!(scene.player_destination_clear([6, 8]).unwrap());
    }

    #[test]
    fn configured_player_height_reaches_an_odd_half_grid_event_across_a_wall() {
        let (mut scene, mut page) = fixture(0, [4, 3], [0, 0], false);
        page.flags = 32;
        scene.set_initial_page(7, Some(&page)).unwrap();
        scene.events.get_mut(&7).unwrap().y2 = 5;
        scene.direction = 6;
        scene.passability.as_mut().unwrap().push(15);
        scene.tiles.as_mut().unwrap()[4 * 10 + 3] = 1;
        let span = Span { start: 0, end: 0 };
        let empty = WolfString { raw: vec![0], span };
        let mut config = wolf_format::GameConfig {
            configuration: vec![0; 37], title: empty.clone(), font: empty.clone(),
            fallback_fonts: vec![], default_character_graphic: empty,
            extra_strings: vec![], declared_size: 0, unknown_size: 0,
            unknown_words: vec![0; 36], internal_offsets: [0; 2],
            opaque_tail: vec![], opaque_tail_span: span,
        };
        config.configuration[0] = 32;
        config.configuration[1] = 4;
        config.configuration[7] = 3;
        config.configuration[8] = 1;
        config.configuration[36] = 1;
        scene.set_game_configuration(&config).unwrap();
        assert!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap().is_empty());
        config.configuration[9] = 1;
        scene.set_game_configuration(&config).unwrap();
        assert!(!scene.player_destination_clear([8, 6]).unwrap());
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap(), [7]);
        assert!(scene.events[&7].route.half_height(), "map actor height stays page-driven");
    }

    #[test]
    fn player_touch_requires_arrival_and_event_touch_rechecks_overlap() {
        let (scene, mut page) = fixture(3, [3, 3], [0, 0], false);
        assert!(scene.map_trigger_candidates(&[(7, &page)], false, false, None).unwrap().is_empty());
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], false, true, None).unwrap(), [7]);
        page.conditions[0] = 4;
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], false, false, None).unwrap(), [7]);
    }

    #[test]
    fn solid_action_and_contact_work_when_a_step_is_blocked() {
        let (mut scene, mut page) = fixture(0, [3, 4], [0, 0], true);
        scene.player_route.set_step_width(2);
        assert!(!scene.player_destination_clear([6, 8]).unwrap());
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap(), [7]);
        page.conditions[0] = 3;
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], false, false, Some(2)).unwrap(), [7]);
    }

    #[test]
    fn counter_extends_confirmation_only_when_its_tileset_flag_is_set() {
        let (mut scene, page) = fixture(0, [3, 5], [0, 0], true);
        scene.player_route.set_step_width(2);
        assert!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap().is_empty());
        scene.passability.as_mut().unwrap()[32] = 0x80;
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap(), [7]);
    }

    #[test]
    fn loop_boundary_can_reach_an_action_on_the_other_edge() {
        let (mut scene, page) = fixture(0, [0, 3], [0, 0], true);
        scene.loop_mode = 1;
        scene.player_x2 = 18;
        scene.direction = 6;
        scene.player_route.set_step_width(2);
        assert_eq!(scene.map_trigger_candidates(&[(7, &page)], true, false, None).unwrap(), [7]);
    }

    #[test]
    fn walking_checks_source_exit_and_target_entry_direction_flags() {
        let (mut scene, _) = fixture(0, [8, 8], [0, 0], false);
        scene.player_route.set_step_width(2);
        scene.passability.as_mut().unwrap().push(1);
        scene.tiles.as_mut().unwrap()[3 * 10 + 3] = 1;
        assert!(scene.player_destination_clear([6, 8]).unwrap());
        assert!(!scene.player_direction_clear([6, 8], 2).unwrap());
        scene.tiles.as_mut().unwrap()[3 * 10 + 3] = 0;
        scene.tiles.as_mut().unwrap()[3 * 10 + 4] = 1;
        scene.passability.as_mut().unwrap()[33] = 8;
        assert!(!scene.player_direction_clear([6, 8], 2).unwrap());
        scene.passability.as_mut().unwrap()[33] = 2;
        assert!(scene.player_direction_clear([6, 8], 2).unwrap());
    }
}

