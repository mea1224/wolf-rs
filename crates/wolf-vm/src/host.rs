//! Bounded, tick-driven host service coordination, independent of a window or GPU.
//! Timing is a Rust runtime policy, not calibrated original-engine behavior.
use crate::runtime::{Completion, Request, Response, Token, Vm};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wolf_ir::Service;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Acknowledge(Token),
    Select(Token, usize),
    Cancel(Token),
    MovementFinished(Token),
    Failed(Token, String),
}
impl Action {
    fn token(&self) -> Token {
        match self {
            Self::Acknowledge(t)
            | Self::Select(t, _)
            | Self::Cancel(t)
            | Self::MovementFinished(t)
            | Self::Failed(t, _) => *t,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Pending {
    request: Request,
    deadline: Option<u64>,
    response: Option<Response>,
}
#[derive(Debug, Default)]
pub struct Poll {
    /// Feed these into the next VM tick (or replay Input.completions).
    pub completions: Vec<Completion>,
    pub rejected_actions: usize,
}
/// Poll after each VM tick. Retains replies until the VM consumes the request.
/// Restore with Host::restore and its paired VM snapshot to preserve deadlines.
/// Old-session actions are rejected after token rebinding.
pub struct Host {
    pending: BTreeMap<Token, Pending>,
    last_tick: Option<u64>,
    max_pending: usize,
    max_actions: usize,
    max_error_bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    tick: u64,
    vm_hash: String,
    max_pending: usize,
    max_actions: usize,
    pending: Vec<Pending>,
}
impl Host {
    /// Save immediately after polling, together with Vm::save(). Responses not
    /// yet consumed and transition deadlines survive restoration.
    pub fn save(&self, vm: &Vm, cap: usize) -> Result<Vec<u8>, String> {
        if self.last_tick != Some(vm.state().tick)
            || self.pending.len() != vm.requests().count()
            || vm.requests().any(|r| !self.pending.contains_key(&r.token))
        {
            return Err("poll host before saving paired state".into());
        }
        // JSON map keys cannot encode compound tokens, hence a sorted vector.
        #[derive(Serialize)]
        struct Borrowed<'a> {
            version: u32,
            tick: u64,
            vm_hash: String,
            max_pending: usize,
            max_actions: usize,
            pending: Vec<&'a Pending>,
        }
        let mut writer = crate::runtime::LimitedWriter::new(cap);
        serde_json::to_writer(
            &mut writer,
            &Borrowed {
                version: 1,
                tick: vm.state().tick,
                vm_hash: vm.state_hash()?,
                max_pending: self.max_pending,
                max_actions: self.max_actions,
                pending: self.pending.values().collect(),
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(writer.into_bytes())
    }

    /// Restore against the paired VM, possibly rebound to a new session.
    pub fn restore(bytes: &[u8], vm: &Vm, cap: usize) -> Result<Self, String> {
        if bytes.len() > cap {
            return Err("host save byte limit".into());
        }
        let saved: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if saved.version != 1
            || saved.tick != vm.state().tick
            || saved.vm_hash != vm.state_hash()?
        {
            return Err("host snapshot does not match VM".into());
        }
        let mut host = Self::new(saved.max_pending, saved.max_actions)?;
        if saved.pending.len() > host.max_pending || saved.pending.len() != vm.requests().count() {
            return Err("host snapshot request count".into());
        }
        for mut pending in saved.pending {
            let request = vm
                .requests()
                .find(|r| r.token.sequence == pending.request.token.sequence)
                .ok_or("host snapshot token mismatch")?;
            if request.service != pending.request.service
                || request.fiber != pending.request.fiber
                || request.page_generation != pending.request.page_generation
                || request.source != pending.request.source
            {
                return Err("host snapshot request mismatch".into());
            }
            let valid_deadline = match request.service {
                Service::Transition { ticks, .. } => pending.deadline.is_some_and(|end| {
                    end <= saved.tick.saturating_add(u64::from(ticks.max(1)))
                        && (pending.response.is_some() || end > saved.tick)
                }),
                _ => pending.deadline.is_none(),
            };
            if !valid_deadline
                || !valid_response(
                    &request.service,
                    pending.response.as_ref(),
                    host.max_error_bytes,
                )
            {
                return Err("host snapshot invalid deadline/response".into());
            }
            pending.request = request.clone();
            if host.pending.insert(request.token, pending).is_some() {
                return Err("host snapshot duplicate token".into());
            }
        }
        host.last_tick = Some(saved.tick);
        Ok(host)
    }
    pub fn new(max_pending: usize, max_actions: usize) -> Result<Self, String> {
        if !(1..=65_536).contains(&max_pending) || !(1..=1_000_000).contains(&max_actions) {
            return Err("host limit outside supported range".into());
        }
        Ok(Self {
            pending: BTreeMap::new(),
            last_tick: None,
            max_pending,
            max_actions,
            max_error_bytes: 4096,
        })
    }
    /// Outstanding UI/backend requests in stable token order. Completed replies
    /// are hidden, even while waiting for the next VM tick to consume them.
    pub fn requests(&self) -> impl Iterator<Item = &Request> {
        self.pending
            .values()
            .filter(|p| p.response.is_none())
            .map(|p| &p.request)
    }
    /// The asset callback must actually load/validate the path, returning Err on
    /// failure. It runs once per token. It must enforce its own byte/path bounds.
    pub fn poll(
        &mut self,
        vm: &Vm,
        actions: &[Action],
        mut load_asset: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<Poll, String> {
        let tick = vm.state().tick;
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err("host clock moved backwards; use a fresh host after restore".into());
        }
        if actions.len() > self.max_actions || vm.requests().count() > self.max_pending {
            return Err("host request/action limit".into());
        }
        // Validate deadlines before pruning requests or performing external work.
        for request in vm.requests() {
            if !self.pending.contains_key(&request.token) {
                if let Service::Transition { ticks, .. } = request.service {
                    tick.checked_add(u64::from(ticks.max(1)))
                        .ok_or("host transition deadline overflow")?;
                }
            }
        }
        let live: BTreeSet<_> = vm.requests().map(|r| r.token).collect();
        self.pending.retain(|token, _| live.contains(token));
        for request in vm.requests() {
            self.pending.entry(request.token).or_insert_with(|| {
                let deadline = match request.service {
                    Service::Transition { ticks, .. } => Some(tick + u64::from(ticks.max(1))),
                    _ => None,
                };
                let response = match &request.service {
                    Service::Asset { path } => Some(match load_asset(path) {
                        Ok(()) => Response::Done,
                        Err(e) => Response::Failed(bounded_error(e, self.max_error_bytes)),
                    }),
                    _ => None,
                };
                Pending {
                    request: request.clone(),
                    deadline,
                    response,
                }
            });
        }
        let mut result = Poll::default();
        for action in actions {
            let Some(pending) = self.pending.get_mut(&action.token()) else {
                result.rejected_actions += 1;
                continue;
            };
            if pending.response.is_some() {
                result.rejected_actions += 1;
                continue;
            }
            let response = match (action, &pending.request.service) {
                (Action::Acknowledge(_), Service::Message { .. }) => Some(Response::Done),
                (Action::Select(_, index), Service::Choice { options, .. })
                    if *index < options.len() =>
                {
                    Some(Response::Choice(*index))
                }
                (
                    Action::Cancel(_),
                    Service::Choice {
                        cancel: Some(_), ..
                    },
                ) => Some(Response::Cancel),
                (Action::MovementFinished(_), Service::Movement { .. }) => Some(Response::Done),
                (Action::Failed(_, e), _) => Some(Response::Failed(bounded_error(
                    e.clone(),
                    self.max_error_bytes,
                ))),
                _ => None,
            };
            if response.is_none() {
                result.rejected_actions += 1;
            }
            pending.response = response;
        }
        for pending in self.pending.values_mut() {
            // A poll prepares input for tick + 1, so a one-tick transition can
            // complete at that boundary. Poll frequency never advances time.
            if pending.response.is_none()
                && pending
                    .deadline
                    .is_some_and(|end| end.saturating_sub(tick) <= 1)
            {
                pending.response = Some(Response::Done);
            }
            if let Some(response) = &pending.response {
                result.completions.push(Completion {
                    token: pending.request.token,
                    response: response.clone(),
                });
            }
        }
        self.last_tick = Some(tick);
        Ok(result)
    }
}
fn bounded_error(mut text: String, cap: usize) -> String {
    if text.len() > cap {
        let mut end = cap;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

fn valid_response(service: &Service, response: Option<&Response>, cap: usize) -> bool {
    match (service, response) {
        (_, None) => true,
        (_, Some(Response::Failed(e))) => e.len() <= cap,
        (Service::Choice { options, .. }, Some(Response::Choice(i))) => *i < options.len(),
        (
            Service::Choice {
                cancel: Some(_), ..
            },
            Some(Response::Cancel),
        ) => true,
        (Service::Choice { .. }, _) => false,
        (_, Some(Response::Done)) => true,
        _ => false,
    }
}
