//! 9F4FB0 continuation queues and 429370 bounded grid search.
use super::{Environment, Geometry};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Search {
    pub goal: [i32; 2],
    nodes: VecDeque<[i32; 2]>,
}

impl Search {
    pub fn new(
        position: [i32; 2],
        goal: [i32; 2],
        half: bool,
        geometry: Geometry,
        host: &impl Environment,
    ) -> Result<Option<Self>, String> {
        if !(0..geometry.size[0]).contains(&goal[0])
            || !(0..geometry.size[1]).contains(&goal[1])
            || !host.path_clear(goal, goal, 0, false)?
        {
            return Ok(None);
        }
        if (position[0] - goal[0] * 2).abs() <= 1 && (position[1] - goal[1] * 2).abs() <= 1 {
            return Ok(Some(Self {
                goal,
                nodes: VecDeque::from([goal]),
            }));
        }
        Ok(search(position, goal, half, geometry, host, false, false)?
            .map(|nodes| Self { goal, nodes }))
    }

    pub fn next(
        &mut self,
        position: [i32; 2],
        half: bool,
        geometry: Geometry,
        host: &impl Environment,
    ) -> Result<[i32; 2], String> {
        // 9F53CE examines at most ten nodes and retains the final destination.
        let mut remove = 0;
        for (i, node) in self.nodes.iter().take(10).enumerate() {
            if (node[0] * 2 - position[0])
                .abs()
                .max((node[1] * 2 - position[1]).abs())
                <= 1
            {
                remove = (i + 1).min(self.nodes.len().saturating_sub(1));
            }
        }
        self.nodes.drain(..remove);
        // 9F5535 searches to the sixth remaining node with event avoidance.
        // That temporary queue is consumed to choose one movement target; it
        // does not replace the retained terrain route to the final goal.
        let waypoint = self
            .nodes
            .get(5)
            .or_else(|| self.nodes.back())
            .copied()
            .unwrap_or(self.goal);
        if let Some(mut local) = search(position, waypoint, half, geometry, host, true, true)? {
            if local.len() > 1
                && local.get(1).is_some_and(|p| {
                    (p[0] * 2 - position[0]).abs() <= 1 && (p[1] * 2 - position[1]).abs() <= 1
                })
            {
                local.pop_front();
            }
            if let Some(next) = local.front() {
                return Ok(next.map(|v| v * 2));
            }
        }
        if self
            .nodes
            .front()
            .is_some_and(|p| p.map(|v| v * 2) == position)
        {
            self.nodes.pop_front();
        }
        Ok(self
            .nodes
            .front()
            .copied()
            .unwrap_or(self.goal)
            .map(|v| v * 2))
    }
}

pub(in super::super) fn direction_delta(direction: i16) -> [i32; 2] {
    match direction {
        1 => [-1, 1],
        2 => [0, 1],
        3 => [1, 1],
        4 => [-1, 0],
        6 => [1, 0],
        7 => [-1, -1],
        8 => [0, -1],
        9 => [1, -1],
        _ => [0, 0],
    }
}

#[derive(Clone, Copy)]
struct Open {
    position: [i32; 2],
    priority: i64,
}

// The native queue compares only its signed 64-bit priority. Preserve its
// right-child tie choice and strict push comparison, rather than introducing
// a coordinate/order tie-breaker through Rust tuple ordering.
fn push(heap: &mut Vec<Open>, node: Open) {
    let mut hole = heap.len();
    heap.push(node);
    while hole > 0 {
        let parent = (hole - 1) / 2;
        if heap[parent].priority <= node.priority {
            break;
        }
        heap[hole] = heap[parent];
        hole = parent;
    }
    heap[hole] = node;
}
fn pop(heap: &mut Vec<Open>) -> Option<Open> {
    let root = *heap.first()?;
    let last = heap.pop().unwrap();
    if heap.is_empty() {
        return Some(root);
    }
    let mut hole = 0;
    while hole * 2 + 1 < heap.len() {
        let left = hole * 2 + 1;
        let right = left + 1;
        let child = if right < heap.len() && heap[right].priority <= heap[left].priority {
            right
        } else {
            left
        };
        heap[hole] = heap[child];
        hole = child;
    }
    while hole > 0 {
        let parent = (hole - 1) / 2;
        if heap[parent].priority <= last.priority {
            break;
        }
        heap[hole] = heap[parent];
        hole = parent;
    }
    heap[hole] = last;
    Some(root)
}

fn search(
    position: [i32; 2],
    goal: [i32; 2],
    half: bool,
    geometry: Geometry,
    host: &impl Environment,
    avoid: bool,
    local: bool,
) -> Result<Option<VecDeque<[i32; 2]>>, String> {
    // Native truncates x/2; non-half actors round positive odd y upward.
    let start = [
        position[0] / 2,
        position[1] / 2 + i32::from(!half && position[1] % 2 == 1),
    ];
    let delta = [
        i64::from(goal[0]) - i64::from(position[0] / 2),
        i64::from(goal[1]) - i64::from(position[1] / 2),
    ];
    // CA83B0 is 500.0; local searches subtract 300 at 9F4D0C.
    let budget = (2 * (delta[0] * delta[0] + delta[1] * delta[1]) + 500
        - if local { 300 } else { 0 })
    .max(0) as usize;
    let directions: &[i16] = if host.directions() == 8 {
        &[8, 9, 6, 3, 2, 1, 4, 7]
    } else {
        &[8, 6, 2, 4]
    };
    let heuristic = |p: [i32; 2]| -> i64 {
        let d: [i64; 2] = std::array::from_fn(|a| {
            let n = i64::from((goal[a] - p[a]).abs());
            if geometry.loops[a] {
                n.min(i64::from(geometry.size[a]) - n)
            } else {
                n
            }
        });
        if host.directions() == 8 {
            d[0] * d[0] + d[1] * d[1]
        } else {
            d[0] + d[1]
        }
    };
    let mut open = Vec::new();
    push(
        &mut open,
        Open {
            position: start,
            priority: heuristic(start),
        },
    );
    let mut distance = BTreeMap::from([(start, 0i64)]);
    let mut previous = BTreeMap::new();
    for _ in 0..budget {
        let Some(current) = pop(&mut open) else {
            return Ok(None);
        };
        if current.position == goal {
            let mut result = VecDeque::from([goal]);
            let mut p = goal;
            while p != start {
                p = *previous.get(&p).ok_or("native path predecessor missing")?;
                result.push_front(p);
                if result.len() > distance.len() {
                    return Err("native path predecessor cycle".into());
                }
            }
            // 9F4EFF removes the search start after reconstructing the path.
            result.pop_front();
            return Ok(Some(result));
        }
        let cost = distance[&current.position];
        for &direction in directions {
            let d = direction_delta(direction);
            let mut next = [current.position[0] + d[0], current.position[1] + d[1]];
            for (a, coordinate) in next.iter_mut().enumerate() {
                if geometry.loops[a] {
                    *coordinate = coordinate.rem_euclid(geometry.size[a]);
                }
            }
            if !(0..geometry.size[0]).contains(&next[0])
                || !(0..geometry.size[1]).contains(&next[1])
            {
                continue;
            }
            if !host.path_clear_goal(current.position, next, direction, avoid, goal)? {
                continue;
            }
            let cost = cost + if d[0] != 0 && d[1] != 0 { 101 } else { 100 };
            if distance.get(&next).is_some_and(|&old| cost >= old) {
                continue;
            }
            distance.insert(next, cost);
            previous.insert(next, current.position);
            push(
                &mut open,
                Open {
                    position: next,
                    priority: cost + heuristic(next),
                },
            );
        }
    }
    Ok(None)
}
