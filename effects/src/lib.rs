//! Optional weapon-effect player. Assets are supplied by the host; no game installation needed.
use anyhow::{Result, bail};
use assets::{
    WeaponModelData, WeaponModelLoadRequest, WeaponVfxAttachments,
    load_weapon_model_from_resource_request, load_weapon_vfx_attachments_from_resource,
};
use scene::{ModelRenderOptions, PreparedModelOptions, WeaponVfxParticles};
pub mod actor;
use scene::ModelRenderer;
mod composite;
pub mod offhand;
mod particles;
mod pose;
pub mod prop;
pub mod timeline;
#[cfg(target_arch = "wasm32")]
mod web;

pub struct EffectAssets {
    model: WeaponModelData,
    options: PreparedModelOptions,
    mounts: WeaponVfxAttachments,
    standalone_rig: Option<assets::ModelSkeleton>,
    standalone_points: Vec<assets::avfx::VfxBindPoint>,
}

impl EffectAssets {
    /// Load in original owner coordinates (ElementId and skeleton transforms use that space).
    pub fn load<R: physis::resource::Resource>(source: &mut R, main: u64) -> Result<Self> {
        let (model, options) = Self::owner(source, main)?;
        let Some(mounts) = load_weapon_vfx_attachments_from_resource(source, &model)? else {
            bail!("no mounted VFX on this weapon variant")
        };
        Ok(Self {
            model,
            options,
            mounts,
            standalone_rig: None,
            standalone_points: Vec::new(),
        })
    }
    /// Load the host for a hand action without requiring any IMC-resident AVFX.
    pub fn load_owner<R: physis::resource::Resource>(source: &mut R, main: u64) -> Result<Self> {
        Self::rigged(source, main, false)
    }
    fn rigged<R: physis::resource::Resource>(
        source: &mut R,
        main: u64,
        include_mounts: bool,
    ) -> Result<Self> {
        let (model, options) = Self::owner(source, main)?;
        let mounts = if include_mounts {
            load_weapon_vfx_attachments_from_resource(source, &model)?.unwrap_or_default()
        } else {
            Default::default()
        };
        let standalone_rig = if mounts.attachments.is_empty() {
            let path = assets::weapon_skeleton_path(model.model_main.model_id);
            let bytes = source
                .read(&path)
                .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
            Some(assets::load_skeleton_from_sklb_bytes(&bytes)?)
        } else {
            anyhow::ensure!(
                pose::owner_skeleton(&mounts).is_some(),
                "no unique held-weapon skeleton"
            );
            None
        };
        let standalone_points = if standalone_rig.is_some() {
            let path = model
                .loaded_paths
                .iter()
                .find(|p| p.ends_with(".mdl"))
                .ok_or_else(|| anyhow::anyhow!("missing weapon MDL"))?;
            pose::mdl_points(&source.read(path)?)?
        } else {
            Vec::new()
        };
        Ok(Self {
            model,
            options,
            mounts,
            standalone_rig,
            standalone_points,
        })
    }
    pub fn owner_path(&self) -> Option<&str> {
        self.model
            .loaded_paths
            .iter()
            .find(|p| p.ends_with(".mdl"))
            .map(String::as_str)
    }
    fn owner<R: physis::resource::Resource>(
        source: &mut R,
        main: u64,
    ) -> Result<(WeaponModelData, PreparedModelOptions)> {
        let model = load_weapon_model_from_resource_request(
            source,
            &WeaponModelLoadRequest {
                item_id: 0,
                item_name: String::new(),
                model_main: main,
                model_sub: 0,
                stain_ids: [0; 2],
            },
        )?;
        let mut options = PreparedModelOptions::default().with_component_preview_layout(false);
        if model.meshes.iter().any(|m| {
            m.submesh
                .as_ref()
                .is_some_and(|s| s.attribute_index_mask != 0)
        }) {
            // The loaded MDL may be a fallback body. Use its actual b#### directory.
            let body = model
                .loaded_paths
                .iter()
                .filter(|p| p.ends_with(".mdl"))
                .find_map(|p| {
                    p.split_once("/obj/body/b")?
                        .1
                        .split('/')
                        .next()?
                        .parse::<u16>()
                        .ok()
                });
            if let Some(body) = body {
                let path = assets::weapon_vfx_imc_path(model.model_main.model_id, body);
                match source.read(&path) {
                    Ok(bytes) => {
                        let imc = assets::ImcFile::parse(&bytes)?;
                        if let Some(entry) =
                            imc.subset_for_variant(model.model_main.variant_id).first()
                        {
                            options = options
                                .with_enabled_attribute_mask(u32::from(entry.attribute_mask()));
                        }
                    }
                    Err(physis::Error::FileNotFound { .. }) => (),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok((model, options))
    }
    pub fn mounts(&self) -> &WeaponVfxAttachments {
        &self.mounts
    }
    pub fn diagnostics(&self) -> Vec<String> {
        self.mounts
            .diagnostic_groups()
            .into_iter()
            .map(|g| scene_note(&g.message))
            .collect()
    }
}

// The asset parser describes its upstream draw backend, not this scene fork.
fn scene_note(note: &str) -> String {
    note.replace("bATM=1 tone-map application is parsed but the VFX forward target has no per-particle tone-map stage",
        "bATM=1 requests scene exposure; forward RGB uses neutral exposure until setParticleTone (projected decals bypass it)")
}

pub struct EffectPlayer {
    random: particles::ParticleEnvironment,
    offhand: Option<offhand::Offhand>,
    actor: Option<actor::Actor>,
    props: Vec<prop::Prop>,
    action: Option<(timeline::ActionPlayer, u64)>,
    renderer: ModelRenderer,
    pose: Option<pose::OwnerPose>,
    standalone_points: Vec<assets::avfx::VfxBindPoint>,
    standalone_elements: Vec<assets::VfxBinderTargetSnapshot>,
    mounts: WeaponVfxAttachments,
    particles: Option<WeaponVfxParticles>,
    composite: composite::Composite,
    resident_visible: bool,
    owner_visible: bool,
    pan: [f32; 2],
    depth: Option<([u32; 2], wgpu::TextureView)>,
    tick: u64,
    resident_origin: u64,
    host_tick: u64,
    remainder: f64,
    material_seconds: f64,
    particle_tone: [f32; 3],
}

pub(crate) fn empty_model_data() -> WeaponModelData {
    WeaponModelData {
        item_id: 0,
        item_name: String::new(),
        model_main: assets::PackedModelId::from_raw(0),
        model_sub: None,
        stain_ids: [0; 2],
        load_diagnostics: Vec::new(),
        loaded_paths: Vec::new(),
        bounds: assets::ModelBounds {
            min: [0.; 3],
            max: [0.; 3],
            center: [0.; 3],
            radius: 1.,
        },
        materials: Vec::new(),
        textures: Vec::new(),
        meshes: Vec::new(),
    }
}

impl EffectPlayer {
    /// Open an empty unified PBR + VFX scene with no mandatory weapon owner.
    pub fn new_empty(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let empty = empty_model_data();
        let renderer = ModelRenderer::new_with_skeleton_and_prepared_options(
            device,
            queue,
            format,
            &empty,
            PreparedModelOptions::default().with_component_preview_layout(false),
            None,
        );
        let mut player = Self::from_renderer(renderer, Default::default());
        player.owner_visible = false;
        player
    }
    pub fn set_pan(&mut self, pan: [f32; 2]) -> Result<()> {
        if !pan.into_iter().all(f32::is_finite) {
            bail!("invalid camera pan");
        }
        self.pan = pan;
        Ok(())
    }
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: EffectAssets,
    ) -> Self {
        let renderer = ModelRenderer::new_with_skeleton_and_prepared_options(
            device,
            queue,
            format,
            &assets.model,
            assets.options,
            pose::owner_skeleton(&assets.mounts).or(assets.standalone_rig.as_ref()),
        );
        let mut player = Self::from_renderer(renderer, assets.mounts);
        if let Some(rig) = assets.standalone_rig {
            player.pose = Some(pose::OwnerPose::from_bind(rig));
            player.standalone_points = assets.standalone_points;
        }
        player
    }
    /// Keep the prepared owner rig but stop drawing the weapon itself.
    pub fn set_owner_visible(&mut self, visible: bool) {
        self.owner_visible = visible;
    }
    fn from_renderer(renderer: ModelRenderer, mounts: WeaponVfxAttachments) -> Self {
        Self {
            random: Default::default(),
            actor: None,
            offhand: None,
            props: Vec::new(),
            action: None,
            renderer,
            pose: pose::OwnerPose::new(&mounts),
            standalone_points: vec![],
            standalone_elements: vec![],
            mounts: Self::forward_depth_mounts(mounts),
            particles: None,
            composite: Default::default(),
            resident_visible: true,
            owner_visible: true,
            pan: [0.; 2],
            depth: None,
            tick: 0,
            resident_origin: 0,
            host_tick: 0,
            remainder: 0.,
            material_seconds: 0.,
            particle_tone: [0., 1., 1.],
        }
    }
    // The pinned scene shader adds DpOf to clip Z. Its projection is forward-Z
    // (LessEqual), so a positive authored pull toward the camera needs a minus.
    // Convert only the consumed playback copy, once per attachment, never on reset.
    fn forward_depth_mounts(mut mounts: WeaponVfxAttachments) -> WeaponVfxAttachments {
        for mount in &mut mounts.attachments {
            particles::prepare_forward_depth(&mut mount.data.file);
        }
        mounts
    }
    pub fn reset(&mut self) {
        self.random.reset();
        self.rewind_actor_partials();
        if let Some(offhand) = &mut self.offhand {
            offhand.reset();
        }
        if let Some((action, start)) = &mut self.action {
            action.reset();
            *start = 0;
        }
        self.particles = None;
        self.tick = 0;
        self.resident_origin = 0;
        self.host_tick = 0;
        self.remainder = 0.;
        self.material_seconds = 0.;
    }
    /// Install a scene RNG snapshot and replay from its initial state.
    pub fn replay_with_random_seed(&mut self, words: [u32; 4]) {
        self.random = particles::ParticleEnvironment::seeded(words);
        if let Some((a, _)) = &mut self.action {
            a.environment = self.random.borrowed();
        }
        self.reset();
    }
    pub fn random_state(&self) -> [u32; 4] {
        self.random.0.snapshot().words()
    }
    /// Solo an action without rewinding resident births or the weapon's pose.
    pub fn set_resident_visible(&mut self, visible: bool) {
        self.resident_visible = visible;
    }
    /// Restart only the resident documents. Host motion and running C012
    /// occurrences keep both their local time and constructor history.
    pub fn replay_residents(&mut self) {
        self.particles = None;
        self.resident_origin = self.tick;
        if let Some(o) = &mut self.offhand {
            o.restart_at(self.tick);
        }
    }
    /// Actual playback fallbacks are distinct from build-time resource diagnostics.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut result = Vec::new();
        if let Some(offhand) = &self.offhand {
            result.extend(offhand.diagnostics());
        }
        if let Some(p) = &self.particles {
            result.extend(
                p.binding_diagnostics()
                    .into_iter()
                    .map(|(path, reason)| format!("{path}: {reason}")),
            );
            result.extend(p.aura_diagnostics().iter().cloned());
        }
        if let Some((action, _)) = &self.action {
            result.extend(action.diagnostics.iter().cloned());
        }
        result
    }
    /// Set the scene's Apricot exposure inputs for subsequent frames. The
    /// sampler value is red at (0.5, 0.5), resolved by the host for this scene.
    pub fn set_particle_tone(&mut self, t_x: f32, t_w: f32, center_red: f32) -> Result<()> {
        let radicand = t_x.mul_add(center_red - 1., 1.);
        if !t_x.is_finite()
            || !t_w.is_finite()
            || !center_red.is_finite()
            || !radicand.is_finite()
            || radicand <= 0.
        {
            bail!("particle tone requires finite inputs and a positive finite rsqrt argument");
        }
        self.particle_tone = [t_x, t_w, center_red];
        Ok(())
    }

    pub fn frame(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        yaw: f32,
        pitch: f32,
        zoom: f32,
        delta: f64,
        enabled: bool,
    ) -> Result<()> {
        self.frame_with_host(
            target,
            size,
            yaw,
            pitch,
            zoom,
            delta,
            enabled,
            |_, _| Ok(()),
        )
    }

    /// Write the host pose before birth and before each fixed simulation step.
    /// The callback's clock is in seconds and may repeat on a zero-delta redraw.
    pub fn frame_with_host(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        yaw: f32,
        pitch: f32,
        zoom: f32,
        delta: f64,
        enabled: bool,
        mut host: impl FnMut(&mut Self, f64) -> Result<()>,
    ) -> Result<()> {
        if size.contains(&0)
            || !delta.is_finite()
            || !yaw.is_finite()
            || !pitch.is_finite()
            || !zoom.is_finite()
        {
            bail!("invalid effect frame")
        }
        // MTRL scroll is measured in seconds and runs even with AVFX hidden.
        self.material_seconds += delta.clamp(0., 0.1);
        let mut opts = ModelRenderOptions {
            uv_scroll_time: self.material_seconds as f32,
            vfx_enabled: enabled,
            ..Default::default()
        };
        host(self, self.host_tick as f64 / 60.)?;
        let advance = |player: &mut Self| -> Result<()> {
            let (ctx, weapon) = player.renderer.scene_parts();
            let instances = scene_models(
                weapon,
                player.actor.as_ref(),
                player.offhand.as_ref(),
                &player.props,
                player.owner_visible,
            );
            let cam =
                ctx.scene_camera_snapshot(&instances, size, yaw, pitch, zoom, player.pan, opts);
            player.prepare_playback(cam)?;
            if let Some(o) = &mut player.offhand {
                o.advance(player.renderer.context(), cam, player.tick, &player.random)?;
            }
            player.advance_documents()
        };
        if enabled {
            advance(self)?;
        }
        self.remainder += delta.clamp(0., 0.1);
        while self.remainder + 1e-10 >= 1. / 60. {
            self.host_tick += 1;
            if enabled {
                self.tick += 1;
            }
            host(self, self.host_tick as f64 / 60.)?;
            if enabled {
                advance(self)?;
            }
            self.remainder -= 1. / 60.;
        }
        self.sample_actor_partials()?;
        // A host callback may have supplied new exposure inputs during this frame.
        opts.particle_tone = self.particle_tone;
        if enabled {
            opts.vfx_time = self.tick as f32 / 60.;
            self.upload_documents((self.tick - self.resident_origin) as f32 / 60.);
            if let Some(o) = &mut self.offhand {
                o.upload(self.renderer.context(), self.tick);
            }
        }
        if self.depth.as_ref().map(|v| v.0) != Some(size) {
            let tex = self
                .renderer
                .device()
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("weapon and effect shared depth"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth24Plus,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
            self.depth = Some((size, tex.create_view(&Default::default())));
        }
        let p = self
            .particles
            .as_ref()
            .filter(|_| enabled && self.resident_visible);
        let offhand_particles = self
            .offhand
            .as_ref()
            .filter(|o| o.visible)
            .and_then(|o| o.playback.as_ref())
            .filter(|_| enabled && self.resident_visible);
        let mut auras = p
            .map(|v| v.unambiguous_active_aura_resources())
            .unwrap_or_default();
        if let Some(o) = offhand_particles {
            auras.extend(o.unambiguous_active_aura_resources());
        }
        let (ctx, weapon) = self.renderer.scene_parts();
        let instances = scene_models(
            weapon,
            self.actor.as_ref(),
            self.offhand.as_ref(),
            &self.props,
            self.owner_visible,
        );
        let mut batches = Vec::with_capacity(3);
        batches.extend(p.map(|v| v.particles()));
        batches.extend(offhand_particles.map(|v| v.particles()));
        batches.extend(self.composite.batch.as_ref().filter(|_| enabled));
        ctx.render_scene_with_vfx_batches(
            &instances,
            target,
            &self.depth.as_ref().unwrap().1,
            size,
            yaw,
            pitch,
            zoom,
            self.pan,
            opts,
            &batches,
            &auras,
        );
        Ok(())
    }
}

fn scene_models<'a>(
    weapon: &'a scene::ModelInstance,
    actor: Option<&'a actor::Actor>,
    offhand: Option<&'a offhand::Offhand>,
    props: &'a [prop::Prop],
    owner_visible: bool,
) -> Vec<&'a scene::ModelInstance> {
    weapon_instances(weapon, owner_visible)
        .chain(actor.map(|a| &a.mesh))
        .chain(offhand.map(|o| &o.mesh))
        .chain(props.iter().filter(|p| p.visible).map(|p| &p.mesh))
        .collect()
}

fn weapon_instances(
    weapon: &scene::ModelInstance,
    owner_visible: bool,
) -> impl Iterator<Item = &scene::ModelInstance> {
    std::iter::once(weapon).filter(move |_| owner_visible)
}
