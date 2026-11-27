use super::{EffectAssets, EffectPlayer};
use assets::resource::MemoryResource;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvas;
use wgpu as w;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen(start)]
pub fn diagnostics() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
}

/// Logical files of one owner + its effect closure, filled by a host's asset resolver.
#[wasm_bindgen]
#[derive(Default)]
pub struct EffectFiles {
    files: HashMap<String, Vec<u8>>,
    bytes: usize,
}
#[wasm_bindgen]
impl EffectFiles {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, path: String, bytes: Vec<u8>) -> Result<(), JsError> {
        if self.files.len() >= crate::actor::MAX_FILES
            || bytes.len() > crate::actor::MAX_FILE_BYTES
            || self.bytes + bytes.len() > crate::actor::MAX_BUNDLE_BYTES
            || self.files.contains_key(&path)
        {
            return Err(err("duplicate or oversized effect bundle"));
        }
        self.bytes += bytes.len();
        self.files.insert(path, bytes);
        Ok(())
    }
}

#[wasm_bindgen]
pub struct DressedActor(super::actor::ActorAssets);

#[wasm_bindgen]
impl DressedActor {
    /// `customize` is the 26-byte ActorCustomize. Equipment uses decimal strings
    /// for the packed IDs, so the JS number representation cannot lose bits.
    pub fn load(
        files: EffectFiles,
        customize: &[u8],
        equipment: &str,
    ) -> Result<DressedActor, JsError> {
        if equipment.len() > 65536 {
            return Err(err("oversized equipment list"));
        }
        let req = crate::actor::ActorRequest {
            customize: customize
                .try_into()
                .map_err(|_| err("expected 26 ActorCustomize bytes"))?,
            equipment: serde_json::from_str(equipment).map_err(err)?,
        }
        .dressed()
        .map_err(err)?;
        super::actor::ActorAssets::load(&mut MemoryResource::new(files.files), &req)
            .map(Self)
            .map_err(err)
    }
    pub fn bone_names(&self) -> Vec<String> {
        self.0.skeleton.bone_names.clone()
    }
    pub fn rest_locals(&self) -> Vec<f32> {
        self.0
            .skeleton
            .rest_pose
            .iter()
            .flat_map(|b| b.translation.into_iter().chain(b.rotation).chain(b.scale))
            .collect()
    }
}

fn actor_locals(
    sample: &js_sys::Function,
    seconds: f64,
    count: usize,
) -> Result<Vec<f32>, JsError> {
    let v = sample
        .call1(&JsValue::UNDEFINED, &seconds.into())
        .map_err(|e| err(format!("actor sampler: {e:?}")))?
        .dyn_into::<js_sys::Float32Array>()
        .map_err(|_| err("expected Float32Array actor pose"))?;
    if v.length() as usize != count * 10 {
        return Err(err("actor pose length mismatch"));
    }
    Ok(v.to_vec())
}

#[wasm_bindgen]
pub struct WeaponEffectViewer {
    actor_motion: Option<js_sys::Function>,
    canvas: OffscreenCanvas,
    owner: assets::WeaponModelData,
    options: scene::PreparedModelOptions,
    surface: w::Surface<'static>,
    device: w::Device,
    config: w::SurfaceConfiguration,
    player: EffectPlayer,
    hand: Option<(js_sys::Function, [f32; 16])>,
    action_hand: Option<(
        js_sys::Function,
        Option<[f32; 16]>,
        Option<crate::timeline::BindTarget>,
    )>,
}
#[wasm_bindgen]
pub fn c010_windows(clips: &str, preview: &str) -> Result<String, JsError> {
    if clips.len() > 4 << 20 || preview.len() > 1 << 20 {
        return Err(err("action manifest too large"));
    }
    let result = crate::timeline::clip_windows(
        serde_json::from_str(clips).map_err(err)?,
        &serde_json::from_str(preview).map_err(err)?,
    )
    .map_err(err)?;
    serde_json::to_string(&result).map_err(err)
}
#[wasm_bindgen]
impl WeaponEffectViewer {
    /// Open an empty unified PBR + VFX scene with no mandatory weapon owner.
    pub async fn create_scene(canvas: OffscreenCanvas) -> Result<Self, JsError> {
        let instance = w::Instance::new(w::InstanceDescriptor {
            backends: w::Backends::BROWSER_WEBGPU,
            ..w::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(w::SurfaceTarget::OffscreenCanvas(canvas.clone()))
            .map_err(err)?;
        let adapter = instance
            .request_adapter(&w::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let (device, queue) = adapter
            .request_device(&w::DeviceDescriptor {
                required_limits: scene::ModelRenderContext::required_limits(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let mut config = surface
            .get_default_config(&adapter, canvas.width().max(1), canvas.height().max(1))
            .ok_or_else(|| err("no WebGPU surface"))?;
        let format = config.format.add_srgb_suffix();
        if format != config.format {
            config.view_formats.push(format);
        }
        surface.configure(&device, &config);
        let player = EffectPlayer::new_empty(device.clone(), queue, format);
        Ok(Self {
            actor_motion: None,
            canvas,
            owner: crate::empty_model_data(),
            options: scene::PreparedModelOptions::default().with_component_preview_layout(false),
            surface,
            device,
            config,
            player,
            hand: None,
            action_hand: None,
        })
    }
    pub async fn create(
        canvas: OffscreenCanvas,
        main: u64,
        files: EffectFiles,
    ) -> Result<Self, JsError> {
        let mut source = MemoryResource::new(files.files);
        let (owner, options) = EffectAssets::owner(&mut source, main).map_err(err)?;
        use physis::resource::Resource;
        let owner_path = owner
            .loaded_paths
            .iter()
            .find(|p| p.ends_with(".mdl"))
            .ok_or_else(|| err("missing owner MDL"))?;
        let points =
            super::pose::mdl_points(&source.read(owner_path).map_err(err)?).map_err(err)?;
        let instance = w::Instance::new(w::InstanceDescriptor {
            backends: w::Backends::BROWSER_WEBGPU,
            ..w::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(w::SurfaceTarget::OffscreenCanvas(canvas.clone()))
            .map_err(err)?;
        let adapter = instance
            .request_adapter(&w::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let (device, queue) = adapter
            .request_device(&w::DeviceDescriptor {
                required_limits: scene::ModelRenderContext::required_limits(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let mut config = surface
            .get_default_config(&adapter, canvas.width().max(1), canvas.height().max(1))
            .ok_or_else(|| err("no WebGPU surface"))?;
        let format = config.format.add_srgb_suffix();
        if format != config.format {
            config.view_formats.push(format);
        }
        surface.configure(&device, &config);
        let renderer = super::ModelRenderer::new_with_skeleton_and_prepared_options(
            device.clone(),
            queue,
            format,
            &owner,
            options.clone(),
            None,
        );
        let mut player = EffectPlayer::from_renderer(renderer, Default::default());
        player.standalone_points = points;
        Ok(Self {
            actor_motion: None,
            canvas,
            owner,
            options,
            surface,
            device,
            config,
            player,
            hand: None,
            action_hand: None,
        })
    }
    pub fn attach(&mut self, files: EffectFiles) -> Result<(), JsError> {
        let mounts = assets::load_weapon_vfx_attachments_from_resource(
            &mut MemoryResource::new(files.files),
            &self.owner,
        )
        .map_err(err)?
        .ok_or_else(|| err("no mounted VFX"))?;
        self.player.renderer.set_model_with_skeleton(
            &self.owner,
            self.options.clone(),
            super::pose::owner_skeleton(&mounts),
        );
        self.player.set_mounts(mounts);
        Ok(())
    }
    /// Fit the owner's skinning buffer without constructing a resident AVFX.
    pub fn prepare_skeleton(&mut self, files: EffectFiles) -> Result<(), JsError> {
        if self.player.pose.is_some() {
            return Ok(());
        }
        let path = assets::weapon_skeleton_path(self.owner.model_main.model_id);
        let bytes = files
            .files
            .get(&path)
            .ok_or_else(|| err(format!("missing {path}")))?;
        let skeleton = assets::load_skeleton_from_sklb_bytes(bytes).map_err(err)?;
        self.player.renderer.set_model_with_skeleton(
            &self.owner,
            self.options.clone(),
            Some(&skeleton),
        );
        self.player.pose = Some(super::pose::OwnerPose::from_bind(skeleton));
        Ok(())
    }
    pub fn set_pan(&mut self, x: f32, y: f32) -> Result<(), JsError> {
        self.player.set_pan([x, y]).map_err(err)
    }
    pub fn add_prop(
        &mut self,
        id: String,
        main: u64,
        bone: String,
        grip: &[f32],
        files: EffectFiles,
    ) -> Result<(), JsError> {
        let grip: [f32; 16] = grip
            .try_into()
            .map_err(|_| err("expected 16-element prop grip matrix"))?;
        let data = crate::prop::PropAssets::load(&mut MemoryResource::new(files.files), main)
            .map_err(err)?;
        self.player.add_prop(id, data, bone, grip).map_err(err)
    }
    pub fn set_prop_visible(&mut self, id: &str, visible: bool) -> Result<(), JsError> {
        self.player.set_prop_visible(id, visible).map_err(err)
    }
    pub fn clear_props(&mut self) {
        self.player.clear_props();
    }
    pub fn set_action_occurrence_enabled(
        &mut self,
        occurrence: usize,
        enabled: bool,
    ) -> Result<(), JsError> {
        self.player
            .action_player_mut()
            .ok_or_else(|| err("no active action"))?
            .set_occurrence_enabled(occurrence, enabled);
        Ok(())
    }
    pub fn seek_action(&mut self, seconds: f64) -> Result<(), JsError> {
        let action_hand = self.action_hand.clone();
        self.player
            .seek_action_with_host(seconds, |player, age| {
                if let Some((sample, grip, target)) = &action_hand {
                    if let Some(grip) = grip {
                        sample_grip(player, sample, grip, age)?;
                    } else {
                        let n = player
                            .actor_skeleton()
                            .ok_or_else(|| anyhow::anyhow!("actor removed"))?
                            .bone_count();
                        player.set_actor_locals(
                            &actor_locals(sample, age, n).map_err(|e| anyhow::anyhow!("{e:?}"))?,
                        )?;
                    }
                    if let Some(target) = target {
                        refresh_action_weapon(player, *target)?;
                    } else if grip.is_none() {
                        player.refresh_equipped_action_weapons()?;
                    }
                }
                Ok(())
            })
            .map_err(err)
    }
    pub fn reset(&mut self) {
        self.player.reset();
    }
    pub fn replay_with_random_seed(&mut self, words: &[u32]) -> Result<(), JsError> {
        self.player.replay_with_random_seed(
            words
                .try_into()
                .map_err(|_| err("expected four RNG words"))?,
        );
        Ok(())
    }
    pub fn random_state(&self) -> Vec<u32> {
        self.player.random_state().to_vec()
    }
    pub fn set_actor_placement(
        &mut self,
        position: &[f32],
        rotation: &[f32],
    ) -> Result<(), JsError> {
        self.player
            .set_actor_placement(
                position.try_into().map_err(|_| err("expected xyz"))?,
                rotation.try_into().map_err(|_| err("expected xyzw"))?,
            )
            .map_err(err)
    }
    pub fn show_residents(&mut self, visible: bool, replay: bool) {
        self.player.set_resident_visible(visible);
        if visible && replay {
            self.player.replay_residents();
        }
    }
    pub fn set_offhand(
        &mut self,
        main: u64,
        owner: &str,
        mounts: &str,
        files: EffectFiles,
        grip: &[f32],
    ) -> Result<(), JsError> {
        let mounts: Vec<String> = serde_json::from_str(mounts).map_err(err)?;
        self.player
            .install_offhand(
                &mut MemoryResource::new(files.files),
                main,
                !mounts.is_empty(),
                super::offhand::OffhandOptions {
                    grip: grip
                        .try_into()
                        .map_err(|_| err("expected 4x4 offhand grip"))?,
                },
                Some((owner, &mounts)),
            )
            .map_err(err)
    }
    pub fn clear_offhand(&mut self) {
        self.player.clear_offhand();
    }
    pub fn offhand_diagnostics(&self) -> Vec<String> {
        self.player
            .offhand
            .as_ref()
            .map_or_else(Vec::new, |o| o.diagnostics())
    }
    pub fn set_owner_visible(&mut self, visible: bool) {
        self.player.set_owner_visible(visible);
    }
    pub fn set_offhand_visible(&mut self, visible: bool) -> Result<(), JsError> {
        self.player.set_offhand_visible(visible).map_err(err)
    }
    pub fn trigger_offhand(&mut self, scheduler: i32, trigger: i32) -> Result<bool, JsError> {
        self.player.trigger_offhand(scheduler, trigger).map_err(err)
    }

    /// Optional actor track; the existing weapon-only API does not need this.
    pub fn set_actor(
        &mut self,
        actor: DressedActor,
        bone: &str,
        grip: &[f32],
        sample: js_sys::Function,
    ) -> Result<(), JsError> {
        let grip = grip
            .try_into()
            .map_err(|_| err("expected 4x4 actor grip"))?;
        let initial = actor_locals(&sample, 0., actor.0.skeleton.bone_count())?;
        self.player
            .install_actor_locals(actor.0, bone, grip, &initial)
            .map_err(err)?;
        self.hand = None;
        self.actor_motion = Some(sample);
        Ok(())
    }
    pub fn clear_actor(&mut self) -> Result<(), JsError> {
        self.player.clear_actor().map_err(err)?;
        self.actor_motion = None;
        Ok(())
    }
    pub fn actor_partial_rigs(&self) -> Vec<String> {
        self.player.actor_partial_rigs()
    }
    pub fn actor_partial_names(&self, path: &str) -> Result<Vec<String>, JsError> {
        self.player
            .actor_partial_skeleton(path)
            .map(|s| s.bone_names.clone())
            .ok_or_else(|| err("unknown partial rig"))
    }
    pub fn actor_partial_rest(&self, path: &str) -> Result<Vec<f32>, JsError> {
        Ok(self
            .player
            .actor_partial_skeleton(path)
            .ok_or_else(|| err("unknown partial rig"))?
            .rest_pose
            .iter()
            .flat_map(|b| b.translation.into_iter().chain(b.rotation).chain(b.scale))
            .collect())
    }
    pub fn set_actor_partial_pose(&mut self, path: &str, locals: &[f32]) -> Result<(), JsError> {
        self.player
            .set_actor_partial_locals(path, locals)
            .map_err(err)
    }
    pub fn clear_actor_partial(&mut self, path: &str) -> Result<(), JsError> {
        self.player.clear_actor_partial(path).map_err(err)
    }
    pub fn set_actor_partial_clip(
        &mut self,
        path: &str,
        pap: &[u8],
        name: &str,
        looping: bool,
    ) -> Result<f32, JsError> {
        self.player
            .set_actor_partial_clip(path, pap, name, looping)
            .map_err(err)
    }
    pub fn play_action(
        &mut self,
        events: &str,
        preview: &str,
        files: EffectFiles,
    ) -> Result<(), JsError> {
        if events.len() > 4 << 20 || preview.len() > 1 << 20 {
            return Err(err("action manifest too large"));
        }
        let (action, draw) = crate::timeline::ActionPlayer::load(
            serde_json::from_str(events).map_err(err)?,
            serde_json::from_str(preview).map_err(err)?,
            |path| {
                files
                    .files
                    .get(path)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("missing action resource: {path}"))
            },
        )
        .map_err(err)?;
        self.player.play_action(action, draw);
        Ok(())
    }
    pub fn stop_action(&mut self) {
        self.player.clear_action();
    }
    pub fn trigger_action(
        &mut self,
        occurrence: usize,
        scheduler: i32,
        trigger: i32,
    ) -> Result<bool, JsError> {
        let Some(p) = self.player.action_player_mut() else {
            return Ok(false);
        };
        p.trigger(occurrence, scheduler, trigger).map_err(err)
    }
    pub fn fade_action(
        &mut self,
        occurrence: usize,
        scheduler: i32,
        frames: i32,
        mode: u32,
        keep: bool,
    ) -> Result<bool, JsError> {
        let Some(p) = self.player.action_player_mut() else {
            return Ok(false);
        };
        p.fade(occurrence, scheduler, frames, mode, keep)
            .map_err(err)
    }
    pub fn trigger_resident(
        &mut self,
        owner_mdl: &str,
        scheduler: i32,
        trigger: i32,
    ) -> Result<bool, JsError> {
        self.player
            .trigger_resident(owner_mdl, scheduler, trigger)
            .map_err(err)
    }
    pub fn action_warning_count(&self) -> usize {
        self.player
            .action
            .as_ref()
            .map_or(0, |(action, _)| action.diagnostics.len())
    }
    pub fn action_finished(&self) -> bool {
        self.player
            .action
            .as_ref()
            .is_some_and(|(action, _)| action.finished())
    }
    pub fn set_controller_input(&mut self, input: &str) -> Result<(), JsError> {
        if input.len() > 4096 {
            return Err(err("oversized C012 controller input"));
        }
        self.player
            .action_player_mut()
            .ok_or_else(|| err("no active action"))?
            .set_controller_input(serde_json::from_str(input).map_err(err)?)
            .map_err(err)
    }
    pub fn controller_status(&self, occurrence: usize) -> Result<String, JsError> {
        serde_json::to_string(
            &self
                .player
                .action
                .as_ref()
                .and_then(|(a, _)| a.controller_status(occurrence)),
        )
        .map_err(err)
    }
    pub fn end_action_commands(&mut self) {
        if let Some(a) = self.player.action_player_mut() {
            a.end_commands();
        }
    }
    pub fn action_warnings_since(&self, count: usize) -> Vec<String> {
        self.player
            .action
            .as_ref()
            .and_then(|(action, _)| action.diagnostics.get(count..))
            .unwrap_or_default()
            .to_vec()
    }
    pub fn preview_seconds(&self) -> f64 {
        action_seconds(&self.player)
    }
    /// Start sampling the action-local clock without rewinding resident or hand clocks.
    pub fn follow_action_hand(
        &mut self,
        sample: js_sys::Function,
        grip: &[f32],
        weapon_selector: &str,
    ) -> Result<(), JsError> {
        let selector = serde_json::from_str(weapon_selector).map_err(err)?;
        let grip: [f32; 16] = grip
            .try_into()
            .map_err(|_| err("expected 4x4 grip matrix"))?;
        sample_grip(&mut self.player, &sample, &grip, 0.).map_err(err)?;
        if let Some(target) = selector {
            refresh_action_weapon(&mut self.player, target).map_err(err)?;
        }
        self.action_hand = Some((sample, Some(grip), selector));
        Ok(())
    }
    pub fn actor_bone_names(&self) -> Vec<String> {
        self.player
            .actor_skeleton()
            .map(|s| s.bone_names.clone())
            .unwrap_or_default()
    }
    pub fn actor_rest_locals(&self) -> Vec<f32> {
        self.player
            .actor_skeleton()
            .into_iter()
            .flat_map(|s| s.rest_pose.iter())
            .flat_map(|b| b.translation.into_iter().chain(b.rotation).chain(b.scale))
            .collect()
    }
    pub fn follow_action_actor(
        &mut self,
        sample: js_sys::Function,
        weapon_selector: &str,
    ) -> Result<(), JsError> {
        let selector = serde_json::from_str(weapon_selector).map_err(err)?;
        let n = self
            .player
            .actor_skeleton()
            .ok_or_else(|| err("no actor attached"))?
            .bone_count();
        self.player
            .set_actor_locals(&actor_locals(&sample, 0., n)?)
            .map_err(err)?;
        if let Some(target) = selector {
            refresh_action_weapon(&mut self.player, target).map_err(err)?;
        }
        self.action_hand = Some((sample, None, selector));
        if selector.is_none() {
            self.player.refresh_equipped_action_weapons().map_err(err)?;
        }
        Ok(())
    }
    pub fn set_action_target(&mut self, binding: &str) -> Result<(), JsError> {
        if binding.len() > 1 << 20 {
            return Err(err("target binding too large"));
        }
        self.player
            .set_action_target(serde_json::from_str(binding).map_err(err)?)
            .map_err(err)
    }
    pub fn set_action_elements(&mut self, selector: &str, elements: &str) -> Result<(), JsError> {
        if elements.len() > 1 << 20 {
            return Err(err("ElementId input too large"));
        }
        let target = serde_json::from_str(selector).map_err(err)?;
        let elements: Vec<crate::timeline::Element> =
            serde_json::from_str(elements).map_err(err)?;
        self.player
            .action
            .as_mut()
            .ok_or_else(|| err("no active action"))?
            .0
            .set_caster_elements(
                target,
                elements
                    .into_iter()
                    .map(|e| assets::VfxBinderTargetSnapshot {
                        id: e.id,
                        matrix: assets::VfxBinderMatrix {
                            basis: e.basis,
                            position: e.position,
                        },
                    })
                    .collect(),
            )
            .map_err(err)
    }
    pub fn set_directional_goal(&mut self, frame: &str) -> Result<(), JsError> {
        if frame.len() > 4096 {
            return Err(err("directional goal input too large"));
        }
        self.player
            .action
            .as_mut()
            .ok_or_else(|| err("no active action"))?
            .0
            .set_directional_goal(serde_json::from_str(frame).map_err(err)?)
            .map_err(err)
    }
    pub fn clear_action_hand(&mut self) -> Result<(), JsError> {
        if self.action_hand.take().is_some() {
            let t = self.player.host_tick as f64 / 60.;
            if let (Some(actor), Some(sample)) = (self.player.actor_skeleton(), &self.actor_motion)
            {
                self.player
                    .set_actor_locals(&actor_locals(sample, t, actor.bone_count())?)
                    .map_err(err)?;
            } else if let Some((sample, grip)) = &self.hand {
                sample_grip(&mut self.player, sample, grip, t).map_err(err)?;
            } else {
                self.player
                    .set_local_pose(&self.player.rest_local_pose())
                    .map_err(err)?;
            }
        }
        Ok(())
    }
    pub fn bone_names(&self) -> Vec<String> {
        self.player.bone_names().to_vec()
    }
    pub fn rest_local_pose(&self) -> Vec<f32> {
        self.player.rest_local_pose()
    }
    pub fn set_local_pose(&mut self, trs: &[f32]) -> Result<(), JsError> {
        self.player.set_local_pose(trs).map_err(err)?;
        self.hand = None;
        self.action_hand = None;
        Ok(())
    }
    /// The callback samples the actor skeleton in the worker's own WASM instance.
    pub fn follow_hand(&mut self, sample: js_sys::Function, grip: &[f32]) -> Result<(), JsError> {
        let grip: [f32; 16] = grip
            .try_into()
            .map_err(|_| err("expected 4x4 grip matrix"))?;
        sample_grip(&mut self.player, &sample, &grip, 0.).map_err(err)?;
        self.hand = Some((sample, grip));
        self.action_hand = None;
        self.player.reset();
        Ok(())
    }
    /// Backpressure the worker's frame loop on GPU completion, not just submission.
    pub fn submitted(&self) -> js_sys::Promise {
        let (tx, rx) = futures_channel::oneshot::channel();
        self.player
            .renderer
            .queue()
            .on_submitted_work_done(move || {
                let _ = tx.send(());
            });
        wasm_bindgen_futures::future_to_promise(async move {
            rx.await.map_err(err)?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn diagnostics(&self) -> Vec<String> {
        self.player.diagnostics()
    }
    pub fn set_particle_tone(
        &mut self,
        t_x: f32,
        t_w: f32,
        center_red: f32,
    ) -> Result<(), JsError> {
        self.player
            .set_particle_tone(t_x, t_w, center_red)
            .map_err(err)
    }
    pub fn frame(
        &mut self,
        yaw: f32,
        pitch: f32,
        zoom: f32,
        delta: f64,
        enabled: bool,
    ) -> Result<(), JsError> {
        let [w, h] = [self.canvas.width().max(1), self.canvas.height().max(1)];
        if w != self.config.width || h != self.config.height {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            w::CurrentSurfaceTexture::Success(v) | w::CurrentSurfaceTexture::Suboptimal(v) => v,
            other => return Err(err(format!("effect surface: {other:?}"))),
        };
        self.player
            .frame_with_host(
                &frame.texture.create_view(&w::TextureViewDescriptor {
                    format: Some(self.config.format.add_srgb_suffix()),
                    ..Default::default()
                }),
                [w, h],
                yaw,
                pitch,
                zoom,
                delta,
                enabled,
                |player, t| {
                    if !matches!(self.action_hand, Some((_, None, _)))
                        && let Some(sample) = &self.actor_motion
                    {
                        let n = player
                            .actor_skeleton()
                            .ok_or_else(|| anyhow::anyhow!("actor removed"))?
                            .bone_count();
                        player.set_actor_locals(
                            &actor_locals(sample, t, n).map_err(|e| anyhow::anyhow!("{e:?}"))?,
                        )?;
                    }
                    if let Some((sample, grip, target)) = &self.action_hand {
                        let age = action_seconds(player);
                        if let Some(grip) = grip {
                            sample_grip(player, sample, grip, age)?;
                        } else {
                            let n = player
                                .actor_skeleton()
                                .ok_or_else(|| anyhow::anyhow!("actor removed"))?
                                .bone_count();
                            player.set_actor_locals(
                                &actor_locals(sample, age, n)
                                    .map_err(|e| anyhow::anyhow!("{e:?}"))?,
                            )?;
                        }
                        if let Some(target) = target {
                            refresh_action_weapon(player, *target)?;
                        } else if grip.is_none() {
                            player.refresh_equipped_action_weapons()?;
                        }
                    } else if let Some((sample, grip)) = &self.hand {
                        sample_grip(player, sample, grip, t)?;
                    }
                    Ok(())
                },
            )
            .map_err(err)?;
        frame.present();
        Ok(())
    }
}
fn refresh_action_weapon(
    player: &mut EffectPlayer,
    target: crate::timeline::BindTarget,
) -> anyhow::Result<()> {
    player.refresh_action_weapon(target)
}
fn action_seconds(player: &EffectPlayer) -> f64 {
    player
        .action
        .as_ref()
        .map_or(0., |(_, origin)| (player.tick - *origin) as f64 / 60.)
}

fn sample_grip(
    player: &mut EffectPlayer,
    sample: &js_sys::Function,
    grip: &[f32; 16],
    t: f64,
) -> anyhow::Result<()> {
    let value = sample
        .call1(&JsValue::UNDEFINED, &t.into())
        .map_err(|e| anyhow::anyhow!("hand sampling failed: {e:?}"))?;
    let value = value
        .dyn_into::<js_sys::Float32Array>()
        .map_err(|_| anyhow::anyhow!("expected Float32Array hand frame"))?;
    let n = value.length() as usize;
    anyhow::ensure!(
        n == 16 || n == 16 + player.bone_names().len() * 10,
        "invalid hand frame length"
    );
    let packet = value.to_vec();
    let hand = (&packet[..16]).try_into().unwrap();
    if n == 16 {
        player.set_weapon_grip(hand, grip)
    } else {
        player.set_posed_weapon_grip(hand, grip, &packet[16..])
    }
}
