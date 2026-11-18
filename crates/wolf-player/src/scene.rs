//! Synthetic IR integration fixture. No original WOLF route opcode mapping.
use std::sync::Arc;
use wolf_ir::{Instruction, Library, Op, Program, Service, Source, Target, PROFILE};
use wolf_media::{Image, Picture, Pictures};
use wolf_vm::{
    host::Host,
    runtime::{Completion, Config, Vm},
    scene::{BlockedPolicy, Direction, Grid, Movement, PageRule, Pages, Position, Step, Trigger},
};

pub struct Scene {
    vm: Vm,
    host: Host,
    movement: Movement,
    pages: Pages,
    completions: Vec<Completion>,
    tiles: Pictures,
    actor: Arc<Image>,
    paused: bool,
}
impl Scene {
    pub fn new() -> Result<Self, String> {
        let code = [
            Op::Request(Service::Movement {
                actor: 1,
                route: vec![],
            }),
            Op::Halt,
        ]
        .into_iter()
        .enumerate()
        .map(|(pc, op)| Instruction {
            source: Source {
                asset: "synthetic-preview".into(),
                content_blake3: "synthetic-v1".into(),
                event: 1,
                page: Some(0),
                command: pc,
                span: [pc, pc + 1],
            },
            raw: None,
            op,
        })
        .collect();
        let library = Library::new(vec![Program {
            id: "preview".into(),
            name: "preview".into(),
            profile: PROFILE.into(),
            code,
        }])?;
        let mut vm = Vm::new(library, Config::default(), 1, 1)?;
        vm.set_global(0, 1)?;
        let mut grid = Grid::new(15, 5, vec![false; 75])?;
        grid.insert(1, Position { x: 1, y: 1 }, true)?;
        let mut pages = Pages::new();
        pages.insert(
            1,
            vec![PageRule {
                target: Target::Id("preview".into()),
                equals: vec![(0, 0)],
                trigger: Trigger::Parallel,
            }],
        )?;
        let mut tiles = Pictures::default();
        for y in 0..5 {
            for x in 0..15 {
                let color = if (x + y) % 2 == 0 {
                    [20, 38, 50, 235]
                } else {
                    [28, 50, 62, 235]
                };
                tiles.show(
                    (y * 15 + x) as u32,
                    Picture {
                        image: Arc::new(Image::new(1, 1, color.to_vec())?),
                        x: 30 + x * 30,
                        y: 350 + y * 30,
                        width: 29,
                        height: 29,
                        opacity: 255,
                        z: 0,
                    },
                )?;
            }
        }
        Ok(Self {
            vm,
            host: Host::new(16, 16)?,
            movement: Movement::new(grid),
            pages,
            completions: vec![],
            tiles,
            actor: Arc::new(Image::new(1, 1, vec![255, 190, 55, 255])?),
            paused: true,
        })
    }
    pub fn toggle(&mut self) -> Result<(), String> {
        self.paused = !self.paused;
        self.vm.set_global(0, i32::from(self.paused))
    }
    pub fn tick(&mut self) -> Result<(), String> {
        let errors = self.pages.update(&mut self.vm, &[]);
        if let Some((_, error)) = errors.into_iter().next() {
            return Err(error);
        }
        self.vm.tick(std::mem::take(&mut self.completions))?;
        let position = self.movement.position(1).ok_or("missing preview actor")?;
        let actions = self.movement.update(&self.vm, |_| {
            let mut steps = vec![];
            // Page cancellation preserves position. Return to the fixture's
            // origin before beginning another rectangle, including mid-route resumes.
            for _ in 1..position.x {
                steps.push(Step::Wait(6));
                steps.push(Step::Move(Direction::Left));
            }
            for _ in 1..position.y {
                steps.push(Step::Wait(6));
                steps.push(Step::Move(Direction::Up));
            }
            for (direction, count) in [
                (Direction::Right, 4),
                (Direction::Down, 2),
                (Direction::Left, 4),
                (Direction::Up, 2),
            ] {
                for _ in 0..count {
                    steps.push(Step::Wait(6));
                    steps.push(Step::Move(direction));
                }
            }
            Ok((steps, BlockedPolicy::Fail))
        })?;
        let poll = self.host.poll(&self.vm, &actions, |_| {
            Err("no asset service in preview".into())
        })?;
        self.completions = poll.completions;
        for fiber in self.vm.state().fibers.values() {
            if let Some(fault) = &fiber.fault {
                return Err(fault.message.clone());
            }
        }
        Ok(())
    }
    pub fn draw(&mut self, image: &mut Image) -> Result<(), String> {
        if self.paused {
            return Ok(());
        }
        let position = self.movement.position(1).ok_or("missing preview actor")?;
        self.tiles.show(
            100,
            Picture {
                image: self.actor.clone(),
                x: 32 + position.x * 30,
                y: 352 + position.y * 30,
                width: 25,
                height: 25,
                opacity: if self.paused { 90 } else { 255 },
                z: 1,
            },
        )?;
        self.tiles.render(image);
        Ok(())
    }
    pub fn tick_number(&self) -> u64 {
        self.vm.state().tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_cancel_freezes_actor_and_reactivation_stays_in_bounds() {
        let mut scene = Scene::new().unwrap();
        for _ in 0..30 {
            scene.toggle().unwrap();
            for _ in 0..39 {
                scene.tick().unwrap();
            }
            scene.toggle().unwrap();
            scene.tick().unwrap();
            let stopped = scene.movement.position(1);
            for _ in 0..20 {
                scene.tick().unwrap();
                assert_eq!(scene.movement.position(1), stopped);
            }
        }
        scene.toggle().unwrap();
        for _ in 0..2000 {
            scene.tick().unwrap();
            let p = scene.movement.position(1).unwrap();
            assert!((1..=5).contains(&p.x) && (1..=3).contains(&p.y));
        }
    }
}
