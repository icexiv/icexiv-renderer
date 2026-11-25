//! C012 command ownership is separate from the AVFX Document lifetime.
//! See tools/c012-evidence.md for the controller-list and trigger trace.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Resolved scene callbacks for one occurrence, never keyed by AVFX path.
/// `None` eligibility means that the optional eligibility lookup was not reached.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerInput {
    pub occurrence: usize,
    #[serde(default = "yes")]
    pub allocation_available: bool,
    #[serde(default)]
    pub eligible: Option<bool>,
    #[serde(default)]
    pub context_code: u32,
    /// True only for a successful object/state lookup with state bit 32 set.
    #[serde(default)]
    pub state_32: bool,
    /// Result of the detached manager's readiness query, not a command window.
    #[serde(default = "yes")]
    pub manager_ready: bool,
}
fn yes() -> bool {
    true
}
impl Default for ControllerInput {
    fn default() -> Self {
        Self {
            occurrence: 0,
            allocation_available: true,
            eligible: None,
            context_code: 0,
            state_32: false,
            manager_ready: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerPhase {
    #[default]
    Waiting,
    AllocationFailed,
    Ineligible,
    StateBlocked,
    Attached,
    Detached,
    Released,
    Skipped,
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ControllerStatus {
    pub phase: ControllerPhase,
    pub external: bool,
    /// Number of distinct trigger-1 inputs queued; identical requests at one
    /// boundary occupy the same bit in the client handle's pending mask.
    pub trigger_requests: u32,
    pub trigger_attached: bool,
    pub state_32_stop: bool,
}

pub(super) struct Controllers {
    flags: Vec<u32>,
    initial: Vec<ControllerInput>,
    input: Vec<ControllerInput>,
    planned: Vec<bool>,
    pub(super) state: Vec<ControllerStatus>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Boundary {
    BeforeStep,
    AfterStep,
    EndCommands,
}
impl Controllers {
    pub(super) fn new(events: &[super::Event], config: Vec<ControllerInput>) -> Result<Self> {
        let mut input: Vec<_> = (0..events.len())
            .map(|occurrence| ControllerInput {
                occurrence,
                ..Default::default()
            })
            .collect();
        let mut seen = vec![false; input.len()];
        for f in config {
            ensure!(
                f.occurrence < input.len() && !seen[f.occurrence],
                "invalid or duplicate controller occurrence {}",
                f.occurrence
            );
            input[f.occurrence] = f;
            seen[f.occurrence] = true;
        }
        Ok(Self {
            flags: events.iter().map(|e| e.visibility).collect(),
            initial: input.clone(),
            input,
            planned: vec![false; events.len()],
            state: events
                .iter()
                .map(|e| ControllerStatus {
                    external: e.visibility & 1 != 0,
                    ..Default::default()
                })
                .collect(),
        })
    }
    pub(super) fn plan(&mut self, cues: &[super::Cue]) {
        for c in cues {
            self.planned[c.event] = true;
        }
        self.reset();
    }
    pub(super) fn reset(&mut self) {
        self.input.clone_from(&self.initial);
        for (s, planned) in self.state.iter_mut().zip(&self.planned) {
            *s = ControllerStatus {
                external: s.external,
                phase: if *planned {
                    ControllerPhase::Waiting
                } else {
                    ControllerPhase::Skipped
                },
                ..Default::default()
            };
        }
    }
    pub(super) fn set(&mut self, f: ControllerInput) -> Result<()> {
        ensure!(
            f.occurrence < self.input.len(),
            "unknown controller occurrence {}",
            f.occurrence
        );
        self.input[f.occurrence] = f;
        Ok(())
    }
    pub(super) fn begin(&mut self, event: usize) -> bool {
        let flags = self.flags[event];
        let f = self.input[event];
        self.state[event].phase = if flags & 1 != 0 && !f.allocation_available {
            ControllerPhase::AllocationFailed
        } else if flags & 4 != 0 && f.state_32 {
            ControllerPhase::StateBlocked
        } else if flags & 2 == 0 && f.eligible == Some(false) && f.context_code != 0xc6 {
            ControllerPhase::Ineligible
        } else {
            ControllerPhase::Attached
        };
        self.state[event].phase == ControllerPhase::Attached
    }
    pub(super) fn sweep(
        &mut self,
        live: &mut Vec<super::Live>,
        cues: &[super::Cue],
        assets: &[super::Asset],
        seconds: f64,
        boundary: Boundary,
        diagnostics: &mut Vec<String>,
    ) {
        live.retain_mut(|l| {
            let cue = &cues[l.cue];
            let event = cue.event;
            let f = self.input[event];
            let s = &mut self.state[event];
            if let Some(at) = l.detached_at {
                if f.manager_ready && seconds > at {
                    s.phase = ControllerPhase::Released;
                    return false;
                }
                return true;
            }
            let state_stop = self.flags[event] & 4 != 0 && f.state_32;
            let expired = seconds >= cue.end && (!s.external || boundary == Boundary::AfterStep);
            if boundary != Boundary::EndCommands && !state_stop && !expired {
                return true;
            }
            s.state_32_stop = state_stop;
            if s.external || state_stop {
                s.trigger_requests += 1;
                // The manager sends (0,1): first registered Scheduler, first
                // trigger slot. No slot is a no-op, not a guessed fade/END.
                if assets[cue.asset]
                    .file
                    .schedulers
                    .iter()
                    .any(|s| !s.triggers.is_empty())
                {
                    match l.playback.trigger_document(0, 1) {
                        Ok(attached) => s.trigger_attached = attached,
                        Err(e) => diagnostics
                            .push(format!("occurrence {event}: controller trigger 1: {e}")),
                    }
                }
            }
            if s.external {
                s.phase = ControllerPhase::Detached;
                l.detached_at = Some(seconds);
                true
            } else {
                s.phase = ControllerPhase::Released;
                false
            }
        });
    }
}
