//! Action preview: one independent AVFX Document per reachable C012 occurrence.
//! The offline importer uses `icexiv_data::tmb`; the pinned playback library is
//! a different revision. Only decoded command records cross that boundary.
mod condition;
pub use condition::ConditionInputs;
mod controller;
mod directional_goal;
use anyhow::{Context, Result, ensure};
use assets::{
    AvfxFile, VfxBinderMatrix, VfxBinderObjectSnapshot, VfxBinderTargetObjectSnapshot,
    VfxBinderTargetSnapshot, VfxDrawModel, VfxMeshInstance, VfxPlayback, VfxQuad,
};
pub use controller::{ControllerInput, ControllerPhase, ControllerStatus};
pub use directional_goal::{DirectionalGoalFrame, DirectionalQuery};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(test)]
mod endpoints_tests;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
pub struct BindTarget {
    pub origin: i8,
    pub kind: i8,
    pub id: i16,
}

#[derive(Debug, Deserialize)]
pub struct Link {
    pub tmb: String,
    pub actor: u16,
    pub track: u16,
    pub command: u16,
    pub actor_start: i16,
    pub track_start: i16,
    pub start: i16,
    pub duration: i32,
    pub ability_delay: i32,
    /// None = no pointer. Each tuple is opcode, integer operand, float operand bits.
    pub conditions: Option<Vec<[u32; 3]>>,
}
/// Common C002/C010/C012 preview policy. Windows are in raw timeline units;
/// playback starts from age zero at the clipped lower bound.
fn window(
    chain: &[Link],
    enabled: &[(String, u16)],
    inputs: Option<&ConditionInputs>,
) -> Result<Option<(i64, i64)>> {
    ensure!(!chain.is_empty() && chain.len() <= 32, "invalid C002 chain");
    let (mut origin, mut lo, mut hi) = (0i64, 0, i64::MAX);
    for l in chain {
        if let Some(script) = l.conditions.as_ref().filter(|s| !s.is_empty()) {
            if let Some(inputs) = inputs.filter(|_| enabled.is_empty()) {
                if !inputs
                    .evaluate(&l.tmb, l.actor, script)
                    .map_err(|e| anyhow::anyhow!("{} track {}: {e}", l.tmb, l.track))?
                {
                    return Ok(None);
                }
            } else {
                ensure!(
                    enabled.iter().any(|(p, id)| p == &l.tmb && *id == l.track),
                    "{} track {} not activated",
                    l.tmb,
                    l.track
                );
            }
        }
        ensure!(
            l.duration > 0 && l.ability_delay == 0,
            "unsupported duration/ability delay"
        );
        origin += i64::from(l.actor_start) + i64::from(l.track_start) + i64::from(l.start);
        lo = lo.max(origin);
        hi = hi.min(origin + i64::from(l.duration));
    }
    Ok((lo < hi).then_some((lo, hi)))
}
#[derive(Deserialize)]
pub struct NamedClip {
    pub chain: Vec<Link>,
    pub name: String,
}
#[derive(serde::Serialize)]
pub struct ClipWindow {
    pub name: String,
    pub start: f64,
    pub end: f64,
    pub occurrence: usize,
}
/// One selected normal PAP pose layer. Overlap is ambiguous; the host must
/// select its actor/tracks explicitly rather than guessing a blend/priority.
pub fn clip_windows(
    clips: Vec<NamedClip>,
    preview: &Preview,
) -> Result<(Vec<ClipWindow>, Vec<String>)> {
    ensure!(
        (1.0..=1000.).contains(&preview.units_per_second) && clips.len() <= 1024,
        "invalid C010 preview clock or count"
    );
    let (mut times, mut notes) = (vec![], vec![]);
    for (occurrence, c) in clips.into_iter().enumerate() {
        match window(
            &c.chain,
            &preview.enabled_tracks,
            preview.condition_inputs.as_ref(),
        ) {
            Ok(Some((lo, hi))) => times.push(ClipWindow {
                name: c.name,
                start: lo as f64 / preview.units_per_second,
                end: hi as f64 / preview.units_per_second,
                occurrence,
            }),
            Ok(None) => (),
            Err(e) => notes.push(format!("C010 {occurrence} ({}): {e}", c.name)),
        }
    }
    times.sort_by(|a, b| a.start.total_cmp(&b.start));
    for (i, pair) in times.windows(2).enumerate() {
        ensure!(
            pair[0].end <= pair[1].start
                || (preview.crossfade_clips
                    && pair[0].start < pair[1].start
                    && pair[0].end < pair[1].end
                    && (i == 0 || times[i - 1].end <= pair[1].start)),
            "overlapping C010 occurrences {} and {}; select one pose layer",
            pair[0].occurrence,
            pair[1].occurrence
        );
    }
    Ok((times, notes))
}
#[derive(Debug, Deserialize)]
pub struct Event {
    /// Outer C002 calls, followed by the C012. Repeated occurrences are distinct.
    pub chain: Vec<Link>,
    pub avfx: String,
    pub targets: [BindTarget; 2],
    pub scale: [f32; 3],
    pub rotation_radians: [f32; 3],
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub visibility: u32,
}
impl Event {
    /// Affine matrix in the C012 creation packet, before external host placement.
    /// See tools/c012-evidence.md. Columns are X/Y/Z, translation is unrotated.
    pub fn command_matrix(&self) -> VfxBinderMatrix {
        let [x, y, z] = self.rotation_radians;
        let q = glam::Quat::from_rotation_z(z)
            * glam::Quat::from_rotation_y(y)
            * glam::Quat::from_rotation_x(x);
        VfxBinderObjectSnapshot {
            position: self.position,
            quaternion: q.to_array(),
            scale: self.scale,
        }
        .matrix()
    }
}
#[derive(Deserialize)]
pub struct Binding {
    pub target: BindTarget,
    /// Caller-resolved world position of this exact caster/target part and ID.
    pub position: [f32; 3],
    #[serde(default = "identity_quaternion")]
    pub quaternion: [f32; 4],
    #[serde(default = "unit_scale")]
    pub scale: [f32; 3],
    /// Resolved AVFX ElementId matrices on this object, in world coordinates.
    /// These IDs are a different namespace from the C012 target's signed ID.
    #[serde(default)]
    pub elements: Vec<Element>,
}
#[derive(Deserialize)]
pub struct Element {
    pub id: u32,
    pub basis: [[f32; 3]; 3],
    pub position: [f32; 3],
}
struct Host {
    selector: BindTarget,
    object: VfxBinderObjectSnapshot,
    elements: Arc<[VfxBinderTargetSnapshot]>,
    second: Arc<VfxBinderTargetObjectSnapshot>,
}
impl Binding {
    fn snapshot(self) -> Result<VfxBinderTargetObjectSnapshot> {
        let object = VfxBinderObjectSnapshot {
            position: self.position,
            quaternion: self.quaternion,
            scale: self.scale,
        };
        object.validate().map_err(anyhow::Error::msg)?;
        let mut ids = std::collections::HashSet::new();
        let targets = self
            .elements
            .into_iter()
            .map(|e| {
                ensure!(
                    ids.insert(e.id)
                        && e.basis
                            .into_iter()
                            .flatten()
                            .chain(e.position)
                            .all(f32::is_finite),
                    "invalid or duplicate ElementId"
                );
                Ok(VfxBinderTargetSnapshot {
                    id: e.id,
                    matrix: VfxBinderMatrix {
                        basis: e.basis,
                        position: e.position,
                    },
                })
            })
            .collect::<Result<_>>()?;
        Ok(VfxBinderTargetObjectSnapshot {
            object: Some(object),
            targets,
            vfx_scale: 1.,
        })
    }
}
fn identity_quaternion() -> [f32; 4] {
    [0., 0., 0., 1.]
}
fn unit_scale() -> [f32; 3] {
    [1.; 3]
}
#[derive(Deserialize)]
pub struct Preview {
    #[serde(default)]
    pub controllers: Vec<ControllerInput>,
    /// Preview policy, not an inferred client clock.
    pub units_per_second: f64,
    /// Host-selected normal-pose transition across the intersection of two
    /// consecutive C010 windows. Nested and triple overlaps remain ambiguous.
    #[serde(default)]
    pub crossfade_clips: bool,
    pub bindings: Vec<Binding>,
    /// Host-evaluated true tracks, keyed by logical TMB path and track ID.
    #[serde(default)]
    pub enabled_tracks: Vec<(String, u16)>,
    /// Opt into native integer expression evaluation at preview admission.
    /// A nonempty explicit track list remains authoritative.
    #[serde(default)]
    pub condition_inputs: Option<ConditionInputs>,
    /// Final Document affine matrix and independent scale supplied by the host.
    /// Keyed by C012 occurrence, never by AVFX path.
    #[serde(default)]
    pub command_frames: Vec<CommandFrame>,
    #[serde(default)]
    pub directional_goals: Vec<DirectionalGoalFrame>,
}
#[derive(Deserialize)]
pub struct CommandFrame {
    pub occurrence: usize,
    pub basis: [[f32; 3]; 3],
    pub position: [f32; 3],
    pub scale: [f32; 3],
}
struct Cue {
    document_color: [f32; 4],
    directional_goal: Option<Arc<VfxBinderTargetObjectSnapshot>>,
    start: f64,
    end: f64,
    asset: usize,
    hosts: [usize; 2],
    event: usize,
    document: VfxBinderMatrix,
    document_scale: Option<[f32; 3]>,
}
struct Asset {
    revised_rgb: [f32; 3],
    directional_file: Option<AvfxFile>,
    file: AvfxFile,
    materials: scene::ParticleMaterials,
    texture_base: i32,
    model_base: usize,
}
struct Live {
    detached_at: Option<f64>,
    cue: usize,
    playback: VfxPlayback,
    caster: Arc<[VfxBinderTargetSnapshot]>,
    target: Arc<VfxBinderTargetObjectSnapshot>,
    age: f32,
}
pub struct ActionPlayer {
    controllers: controller::Controllers,
    pub(crate) environment: crate::particles::ParticleEnvironment,
    hosts: Vec<Host>,
    view: Option<assets::VfxCameraViewSnapshot>,
    pub events: Vec<Event>,
    cues: Vec<Cue>,
    assets: Vec<Asset>,
    live: Vec<Live>,
    disabled_occurrences: std::collections::HashSet<usize>,
    next: usize,
    time: f64,
    pub diagnostics: Vec<String>,
    initial_diagnostics: usize,
}
pub struct DrawAssets {
    pub textures: Vec<Option<scene::VfxTextureInput>>,
    pub models: Vec<VfxDrawModel>,
}
impl ActionPlayer {
    pub(crate) fn has_binding(&self, target: BindTarget) -> bool {
        self.hosts.iter().any(|h| h.selector == target)
    }
    /// Preview policy: additive local starts, half-open command windows clipped
    /// by C002 ancestors. Nonpositive durations and ability delays are unresolved.
    pub fn load(
        events: Vec<Event>,
        config: Preview,
        mut read: impl FnMut(&str) -> Result<Vec<u8>>,
    ) -> Result<(Self, DrawAssets)> {
        ensure!(
            config.units_per_second.is_finite() && (1.0..=1000.).contains(&config.units_per_second),
            "preview clock must be 1..=1000 units/s"
        );
        ensure!(events.len() <= 1024, "timeline event budget");
        let mut controllers = controller::Controllers::new(&events, config.controllers)?;
        let mut frames = HashMap::new();
        for f in config.command_frames {
            ensure!(
                f.occurrence < events.len()
                    && f.basis
                        .into_iter()
                        .flatten()
                        .chain(f.position)
                        .chain(f.scale)
                        .all(f32::is_finite),
                "invalid C012 command frame"
            );
            ensure!(
                frames
                    .insert(
                        f.occurrence,
                        (
                            VfxBinderMatrix {
                                basis: f.basis,
                                position: f.position
                            },
                            f.scale
                        )
                    )
                    .is_none(),
                "duplicate C012 command frame"
            );
        }
        let mut bindings = HashMap::new();
        let mut hosts = vec![];
        for binding in config.bindings {
            let selector = binding.target;
            let second = Arc::new(binding.snapshot()?);
            ensure!(
                bindings.insert(selector, hosts.len()).is_none(),
                "duplicate C012 binding"
            );
            hosts.push(Host {
                selector,
                object: second.object.unwrap(),
                elements: second.targets.clone().into(),
                second,
            });
        }
        let mut draw = DrawAssets {
            textures: vec![],
            models: vec![],
        };
        let mut directional = HashMap::new();
        for goal in config.directional_goals {
            ensure!(
                goal.occurrence < events.len()
                    && directional
                        .insert(goal.occurrence, goal.resolved()?)
                        .is_none(),
                "invalid or duplicate directional goal occurrence"
            );
        }
        let mut by_path = HashMap::new();
        let mut assets = Vec::new();
        let mut cues = Vec::new();
        let mut diagnostics = Vec::new();
        for (event, e) in events.iter().enumerate() {
            ensure!(
                !e.chain.is_empty() && e.chain.len() <= 32,
                "invalid C002 chain"
            );
            let (lower, upper) = match window(
                &e.chain,
                &config.enabled_tracks,
                config.condition_inputs.as_ref(),
            ) {
                Ok(Some(w)) => w,
                Ok(None) => continue,
                Err(reason) => {
                    diagnostics.push(format!("occurrence {event}: {reason}"));
                    continue;
                }
            };
            if e.visibility & !7 != 0 {
                diagnostics.push(format!(
                    "occurrence {event}: unknown C012 controller flags {:#x}",
                    e.visibility
                ));
                continue;
            }
            let [Some(a), Some(b)] = e.targets.map(|t| bindings.get(&t).copied()) else {
                diagnostics.push(format!(
                    "occurrence {event}: unresolved C012 endpoints {:?}",
                    e.targets
                ));
                continue;
            };
            let host = hosts[a].object.matrix();
            let document = if let Some((m, _)) = frames.get(&event) {
                *m
            } else if e.scale == [1.; 3] && e.position == [0.; 3] && e.rotation_radians == [0.; 3] {
                host
            } else if host == VfxBinderObjectSnapshot::IDENTITY.matrix() {
                let m = e.command_matrix();
                ensure!(
                    m.basis
                        .into_iter()
                        .flatten()
                        .chain(m.position)
                        .all(f32::is_finite),
                    "invalid C012 command matrix"
                );
                m
            } else {
                diagnostics.push(format!("occurrence {event}: nondefault C012 TRS with nonidentity host placement requires a resolved command frame"));
                continue;
            };
            let asset = if let Some(&i) = by_path.get(&e.avfx) {
                i
            } else {
                let mut file = AvfxFile::parse(&read(&e.avfx)?).with_context(|| e.avfx.clone())?;
                diagnostics.extend(
                    file.warnings
                        .iter()
                        .map(|w| format!("{}: {}", e.avfx, crate::scene_note(w))),
                );
                for (i, binder) in file
                    .binders
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| matches!(b.binder_type, 1 | 4))
                {
                    for (end, p) in [
                        ("start", &binder.properties_start),
                        ("goal", &binder.properties_goal),
                    ] {
                        if let Some(p) = p
                            && !matches!(p.bind_target_point_type, 0 | 3)
                            && !directional_goal::eligible(&file)
                        {
                            diagnostics.push(format!("{}: Linear binder {i} {end} BPTP={} is not supported by staged target queries (object=0, ElementId=3)",e.avfx,p.bind_target_point_type));
                        }
                    }
                }
                crate::particles::prepare_forward_depth(&mut file);
                let materials = scene::ParticleMaterials::prepare(&mut file);
                // Both staged and fallback samplers multiply this into particle
                // colors. Action draws supply it in the Document PS constant.
                let revised_rgb = std::mem::replace(&mut file.global.revised_color, [1.; 3]);
                let texture_base = draw.textures.len() as i32;
                let model_base = draw.models.len();
                for path in &file.texture_paths {
                    let texture = read(path)
                        .ok()
                        .and_then(|bytes| assets::decode_atex_rgba(&bytes));
                    if texture.is_none() {
                        diagnostics.push(format!("missing or invalid texture: {path}"));
                    }
                    draw.textures.push(texture.as_ref().map(Into::into));
                }
                draw.models.extend(
                    file.models
                        .iter()
                        .map(|m| m.draw.clone().unwrap_or_default()),
                );
                let i = assets.len();
                assets.push(Asset {
                    revised_rgb,
                    directional_file: None,
                    file,
                    materials,
                    texture_base,
                    model_base,
                });
                by_path.insert(e.avfx.clone(), i);
                i
            };
            let goal = directional.remove(&event);
            let asset_ref = &mut assets[asset];
            if goal.is_some() {
                ensure!(
                    directional_goal::eligible(&asset_ref.file)
                        && e.targets[1]
                            == (BindTarget {
                                origin: 2,
                                kind: 0,
                                id: -1
                            }),
                    "directional goal does not match the unoverridden target-list Linear query"
                );
                asset_ref.directional_file.get_or_insert_with(|| {
                    let mut copy = asset_ref.file.clone();
                    let point = copy.binders[0].properties_goal.as_mut().unwrap();
                    point.bind_target_point_type = 3;
                    point.bind_point_id = directional_goal::QUERY_SLOT;
                    copy
                });
            } else if directional_goal::eligible(&asset_ref.file) {
                diagnostics.push(format!("occurrence {event}: missing resolved Linear binder 0 goal BPTP=2 frame for target-list ordinal 0; original AVFX retained"));
            }
            let [r, g, blue, alpha] = e.color;
            let [rr, rg, rb] = asset_ref.revised_rgb;
            let document_color = [rr * r, rg * g, rb * blue, alpha];
            ensure!(
                document_color.into_iter().all(f32::is_finite),
                "occurrence {event}: nonfinite Document color"
            );
            cues.push(Cue {
                document_color,
                directional_goal: goal,
                asset,
                hosts: [a, b],
                start: lower as f64 / config.units_per_second,
                end: upper as f64 / config.units_per_second,
                event,
                document,
                document_scale: frames.get(&event).map(|f| f.1),
            });
        }
        cues.sort_by(|a, b| a.start.total_cmp(&b.start));
        controllers.plan(&cues);
        Ok((
            Self {
                controllers,
                environment: Default::default(),
                hosts,
                view: None,
                events,
                cues,
                assets,
                live: vec![],
                disabled_occurrences: Default::default(),
                next: 0,
                time: 0.,
                initial_diagnostics: diagnostics.len(),
                diagnostics,
            },
            draw,
        ))
    }
    pub fn reset(&mut self) {
        self.controllers.reset();
        self.live.clear();
        self.environment.reset();
        self.next = 0;
        self.time = 0.;
        self.diagnostics.truncate(self.initial_diagnostics);
    }
    /// World-space camera shared by action Documents. Install before birth so
    /// a Camera binder never has to reconstruct its constructor cache.
    pub fn set_camera_view(&mut self, view: assets::VfxCameraViewSnapshot) -> Result<()> {
        view.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.view.is_some() || self.live.is_empty(),
            "supply initial action camera before Document birth"
        );
        self.view = Some(view);
        Ok(())
    }
    /// Replace the first binding's resolved AVFX ElementId provider. Newborns
    /// and supported live binder roots see it at the next simulation input.
    pub fn set_caster_elements(
        &mut self,
        target: BindTarget,
        elements: Vec<VfxBinderTargetSnapshot>,
    ) -> Result<()> {
        let index = self
            .hosts
            .iter()
            .position(|h| h.selector == target)
            .context("unknown C012 endpoint")?;
        let mut ids = std::collections::HashSet::new();
        ensure!(
            elements.iter().all(|e| ids.insert(e.id)
                && e.matrix
                    .basis
                    .into_iter()
                    .flatten()
                    .chain(e.matrix.position)
                    .all(f32::is_finite)),
            "invalid or duplicate ElementId"
        );
        let host = &mut self.hosts[index];
        if *host.elements != elements {
            host.elements = elements.into();
        }
        Ok(())
    }
    /// Queue one complete world-space second endpoint (object plus ElementIds).
    /// New documents use it at birth; a live document installs the latest
    /// snapshot before its next positive input. It does not move the caster.
    pub fn set_target_binding(&mut self, binding: Binding) -> Result<()> {
        let index = self
            .hosts
            .iter()
            .position(|h| h.selector == binding.target)
            .context("unknown C012 endpoint")?;
        ensure!(
            self.cues.iter().any(|c| c.hosts[1] == index),
            "selector is not an active-plan C012 second endpoint"
        );
        let snapshot = binding.snapshot()?;
        if *self.hosts[index].second != snapshot {
            self.hosts[index].second = Arc::new(snapshot);
        }
        Ok(())
    }
    pub fn active_occurrences(&self) -> impl Iterator<Item = usize> + '_ {
        self.live
            .iter()
            .filter(|l| l.detached_at.is_none())
            .map(|l| self.cues[l.cue].event)
    }
    pub fn set_controller_input(&mut self, input: ControllerInput) -> Result<()> {
        self.controllers.set(input)
    }
    pub fn controller_status(&self, occurrence: usize) -> Option<ControllerStatus> {
        self.controllers.state.get(occurrence).copied()
    }
    /// Graceful command teardown. External Documents may still sample/advance;
    /// `finished()` includes their eventual manager release. Dropping this
    /// player instead disposes the whole preview, including its manager.
    pub fn end_commands(&mut self) {
        for c in &self.cues[self.next..] {
            self.controllers.state[c.event].phase = ControllerPhase::Skipped;
        }
        self.next = self.cues.len();
        self.controllers.sweep(
            &mut self.live,
            &self.cues,
            &self.assets,
            self.time,
            controller::Boundary::EndCommands,
            &mut self.diagnostics,
        );
    }
    /// Refresh the same successful directional query. For initialization, pass
    /// `Preview::directional_goals`; an already-born fallback is never rebuilt.
    pub fn set_directional_goal(&mut self, frame: DirectionalGoalFrame) -> Result<()> {
        let goal = frame.resolved()?;
        let cue = self
            .cues
            .iter_mut()
            .find(|c| c.event == frame.occurrence)
            .context("unknown directional goal occurrence")?;
        let current = cue
            .directional_goal
            .as_mut()
            .context("directional goal must be provided before playback")?;
        if **current != *goal {
            *current = goal;
        }
        Ok(())
    }
    fn document_mut(&mut self, occurrence: usize) -> Option<&mut VfxPlayback> {
        self.live
            .iter_mut()
            .find(|l| l.detached_at.is_none() && self.cues[l.cue].event == occurrence)
            .map(|l| &mut l.playback)
    }
    /// Apply a host event to this occurrence only, even if another live cue
    /// references the same AVFX. A trigger is 1-based; scheduler is 0-based.
    pub fn trigger(&mut self, occurrence: usize, scheduler: i32, trigger: i32) -> Result<bool> {
        let Some(doc) = self.document_mut(occurrence) else {
            return Ok(false);
        };
        doc.trigger_document(scheduler, trigger)
            .map_err(anyhow::Error::msg)
    }
    /// Fade modes: 0 hold, 1 in, 2 out, 3 out and stop ordinary factories.
    pub fn fade(
        &mut self,
        occurrence: usize,
        scheduler: i32,
        frames: i32,
        mode: u32,
        inhibit_retirement: bool,
    ) -> Result<bool> {
        ensure!(frames >= 0 && mode < 4, "invalid scheduler fade");
        let Some(doc) = self.document_mut(occurrence) else {
            return Ok(false);
        };
        doc.configure_scheduler_fade(scheduler, frames, mode, inhibit_retirement)
            .map_err(anyhow::Error::msg)
    }
    pub fn set_occurrence_enabled(&mut self, occurrence: usize, enabled: bool) {
        if enabled {
            self.disabled_occurrences.remove(&occurrence);
        } else {
            self.disabled_occurrences.insert(occurrence);
        }
    }
    pub fn finished(&self) -> bool {
        self.next == self.cues.len() && self.live.is_empty()
    }
    /// Monotonic seconds; callers advance at a fixed simulation cadence.
    pub fn advance_to(&mut self, seconds: f64) -> Result<()> {
        ensure!(
            seconds.is_finite() && seconds >= self.time,
            "backward or invalid action clock"
        );
        // A trigger queued at the end of a live external command must never
        // receive time from *before* that boundary, even on a coarse host step.
        while let Some(t) = self
            .live
            .iter()
            .filter(|l| l.detached_at.is_none())
            .map(|l| &self.cues[l.cue])
            .filter(|c| {
                self.events[c.event].visibility & 1 != 0 && c.end > self.time && c.end < seconds
            })
            .map(|c| c.end)
            .min_by(f64::total_cmp)
        {
            self.advance_slice(t)?;
        }
        self.advance_slice(seconds)
    }
    fn advance_slice(&mut self, seconds: f64) -> Result<()> {
        self.controllers.sweep(
            &mut self.live,
            &self.cues,
            &self.assets,
            seconds,
            controller::Boundary::BeforeStep,
            &mut self.diagnostics,
        );
        while let Some(c) = self.cues.get(self.next).filter(|c| c.start <= seconds) {
            let index = self.next;
            self.next += 1;
            if seconds >= c.end {
                self.controllers.state[c.event].phase = ControllerPhase::Skipped;
                continue;
            }
            if !self.controllers.begin(c.event) {
                continue;
            }
            let [first, second] = c.hosts.map(|i| &self.hosts[i]);
            let asset = &self.assets[c.asset];
            let (file, target) = if let Some(goal) = &c.directional_goal {
                (
                    asset
                        .directional_file
                        .as_ref()
                        .expect("validated directional goal"),
                    goal,
                )
            } else {
                (&asset.file, &second.second)
            };
            let runtime = self
                .environment
                .runtime(file)
                .with_document_matrix(c.document)
                .and_then(|r| {
                    if let Some(s) = c.document_scale {
                        r.with_document_scale(s)
                    } else {
                        Ok(r)
                    }
                })
                .and_then(|r| r.with_binder_object_snapshot(first.object))
                .and_then(|r| r.with_binder_target_snapshot(&first.elements))
                .and_then(|r| r.with_binder_target_objects(std::slice::from_ref(&**target)))
                .map_err(anyhow::Error::msg)?;
            let runtime = if let Some(view) = self.view {
                // Do not synthesize client Euler fields from a rotated quaternion.
                let runtime = if first.object.quaternion == identity_quaternion() {
                    runtime
                        .with_preview_camera_host_rotation([0.; 3])
                        .map_err(anyhow::Error::msg)?
                } else {
                    runtime
                };
                runtime
                    .with_preview_camera_view(view)
                    .map_err(anyhow::Error::msg)?
            } else {
                runtime
            };
            let mut playback = VfxPlayback::new(runtime);
            playback.advance(0.).map_err(anyhow::Error::msg)?;
            self.live.push(Live {
                detached_at: None,
                cue: index,
                playback,
                caster: Arc::clone(&first.elements),
                target: Arc::clone(target),
                age: 0.,
            });
        }
        for l in &mut self.live {
            let cue = &self.cues[l.cue];
            let age = (seconds - cue.start) as f32;
            let [first, second] = cue.hosts.map(|i| &self.hosts[i]);
            let changed = (!Arc::ptr_eq(&l.caster, &first.elements) && l.caster != first.elements)
                .then_some(&*first.elements);
            let target = cue.directional_goal.as_ref().unwrap_or(&second.second);
            let new_target = !Arc::ptr_eq(&l.target, target) && l.target != *target;
            if new_target && age > l.age {
                // The target-list adapter cannot install a camera/caster. Give
                // changed pose inputs one old-age Prepare, then let only the
                // target-list adapter consume the positive delta.
                if let Some(view) = self.view {
                    l.playback
                        .update_preview_camera_view_to(l.age, changed, view)
                        .map_err(anyhow::Error::msg)?;
                    l.caster = Arc::clone(&first.elements);
                } else if let Some(elements) = changed {
                    if l.playback.fallback_reason().is_none() {
                        l.playback
                            .advance_with_binder_target_snapshot(0., elements)
                            .map_err(anyhow::Error::msg)?;
                        l.caster = Arc::clone(&first.elements);
                    } else {
                        // No equal-age caster-only setter in continuous mode.
                        // Keep the caster update pending for the next input.
                        let msg = format!(
                            "occurrence {}: concurrent caster update deferred in camera-less fallback",
                            cue.event
                        );
                        if !self.diagnostics.contains(&msg) {
                            self.diagnostics.push(msg);
                        }
                    }
                }
                l.playback
                    .update_to_with_binder_target_objects(age, std::slice::from_ref(&**target))
                    .map_err(anyhow::Error::msg)?;
                l.target = Arc::clone(target);
            } else if let Some(view) = self.view {
                l.playback
                    .update_preview_camera_view_to(age, changed, view)
                    .map_err(anyhow::Error::msg)?;
                l.caster = Arc::clone(&first.elements);
            } else if let Some(targets) = changed {
                if age == l.age {
                    continue;
                }
                l.playback
                    .update_to_with_binder_target_snapshot(age, targets)
                    .map_err(anyhow::Error::msg)?;
                l.caster = Arc::clone(&first.elements);
            } else {
                l.playback.update_to(age);
            }
            l.age = age;
            if let Some(reason) = l.playback.fallback_reason() {
                let msg = format!("occurrence {}: {reason}", self.cues[l.cue].event);
                if !self.diagnostics.contains(&msg) {
                    self.diagnostics.push(msg);
                }
            }
        }
        self.time = seconds;
        self.controllers.sweep(
            &mut self.live,
            &self.cues,
            &self.assets,
            seconds,
            controller::Boundary::AfterStep,
            &mut self.diagnostics,
        );
        Ok(())
    }
    /// Geometry and particle colors before fragment-stage Document modulation.
    pub fn sample(&self) -> (Vec<VfxQuad>, Vec<VfxMeshInstance>) {
        let (mut quads, mut meshes) = (vec![], vec![]);
        let mut namespace = crate::particles::PacketNamespace::default();
        self.visit_packets(|_, _, _, definitions, qs, ms, _, _, _| {
            namespace.rebase(qs, ms, definitions);
            quads.extend_from_slice(&qs[..qs.len().min(4096 - quads.len())]);
            meshes.extend_from_slice(&ms[..ms.len().min(4096 - meshes.len())]);
        });
        (quads, meshes)
    }
    pub(crate) fn visit_packets(
        &self,
        mut f: impl FnMut(
            [f32; 3],
            f32,
            usize,
            usize,
            &mut [VfxQuad],
            &mut [VfxMeshInstance],
            &[bool],
            &[bool],
            [f32; 4],
        ),
    ) {
        let (mut qs, mut ms) = (vec![], vec![]);
        for l in &self.live {
            let c = &self.cues[l.cue];
            if self.disabled_occurrences.contains(&c.event) {
                continue;
            }
            let a = &self.assets[c.asset];
            l.playback.sample(&mut qs, &mut ms);
            ms.retain(|m| m.model_index < a.file.models.len());
            let q_materials: Vec<_> = qs.iter().map(|q| a.materials.quad(q)).collect();
            let m_materials: Vec<_> = ms.iter_mut().map(|m| a.materials.mesh(m)).collect();
            let tex = |i: i32| {
                if i < 0 {
                    i
                } else if (i as usize) < a.file.texture_paths.len() {
                    a.texture_base + i
                } else {
                    i32::MAX
                }
            };
            for q in &mut qs {
                q.texture_indexes = q.texture_indexes.map(tex);
                q.texture_palette_index = tex(q.texture_palette_index);
                q.texture_distortion_index = tex(q.texture_distortion_index);
            }
            for m in &mut ms {
                m.model_index += a.model_base;
                m.texture_indexes = m.texture_indexes.map(tex);
                m.texture_distortion_index = tex(m.texture_distortion_index);
                m.texture_normal_index = tex(m.texture_normal_index);
                m.texture_palette_index = tex(m.texture_palette_index);
                m.reflection_texture_index = tex(m.reflection_texture_index);
            }
            f(
                c.document.position,
                a.file.global.soft_key_offset,
                l.cue,
                a.file.particles.len(),
                &mut qs,
                &mut ms,
                &q_materials,
                &m_materials,
                c.document_color,
            );
        }
    }
}
