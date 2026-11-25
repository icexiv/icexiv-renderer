//! A second, independently loaded weapon instance in the actor's left hand.
use super::{EffectAssets, EffectPlayer, actor, pose::OwnerPose};
use anyhow::{Context, Result, ensure};
use assets::{VfxBinderTargetSnapshot, VfxCameraViewSnapshot, WeaponVfxAttachments};
use glam::{Mat4, Vec3};
use physis::resource::Resource;
use scene::{ModelInstance, ModelRenderContext, WeaponVfxParticles};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

/// Manifest emitted by `export` (resident) or `export --owner` (no resident).
#[derive(Deserialize)]
pub struct OffhandManifest {
    pub main: String,
    pub owner: String,
    pub files: BTreeMap<String, String>,
    pub mounts: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct OffhandOptions {
    /// Column-major matrix mapping weapon coordinates to `n_buki_l` local space.
    pub grip: [f32; 16],
}
impl Default for OffhandOptions {
    fn default() -> Self {
        Self {
            grip: Mat4::IDENTITY.to_cols_array(),
        }
    }
}

pub(super) struct Offhand {
    pub(super) visible: bool,
    pub(super) mesh: ModelInstance,
    pub(super) playback: Option<WeaponVfxParticles>,
    mounts: WeaponVfxAttachments,
    bind: OwnerPose,
    rest: Vec<f32>,
    bind_sphere: ([f32; 3], f32),
    points: Vec<assets::avfx::VfxBindPoint>,
    elements: Vec<VfxBinderTargetSnapshot>,
    grip: [f32; 16],
    last_hand: Option<[f32; 16]>,
    origin: u64,
    owner: String,
}

impl EffectPlayer {
    pub fn set_offhand_bundle(
        &mut self,
        manifest: &OffhandManifest,
        mut read: impl FnMut(&str, &str) -> Result<Vec<u8>>,
        options: OffhandOptions,
    ) -> Result<()> {
        ensure!(
            manifest.files.len() <= actor::MAX_FILES,
            "too many offhand resources"
        );
        ensure!(
            manifest.files.contains_key(&manifest.owner),
            "missing offhand owner entry"
        );
        let mut files = HashMap::new();
        let mut total = 0usize;
        for (path, hash) in &manifest.files {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid offhand CAS hash"
            );
            let data = read(path, hash).with_context(|| path.clone())?;
            total = total
                .checked_add(data.len())
                .context("offhand size overflow")?;
            ensure!(
                data.len() <= actor::MAX_FILE_BYTES && total <= actor::MAX_BUNDLE_BYTES,
                "oversized offhand bundle"
            );
            files.insert(path.clone(), data);
        }
        self.install_offhand(
            &mut assets::resource::MemoryResource::new(files),
            manifest.main.parse()?,
            !manifest.mounts.is_empty(),
            options,
            Some((&manifest.owner, &manifest.mounts)),
        )
    }

    /// Native path for an existing SqPack or in-memory resource provider.
    pub fn set_offhand<R: Resource>(
        &mut self,
        source: &mut R,
        main: u64,
        resident: bool,
        options: OffhandOptions,
    ) -> Result<()> {
        self.install_offhand(source, main, resident, options, None)
    }

    pub(super) fn install_offhand<R: Resource>(
        &mut self,
        source: &mut R,
        main: u64,
        resident: bool,
        options: OffhandOptions,
        expected: Option<(&str, &[String])>,
    ) -> Result<()> {
        let hand = self
            .left_hand_world()
            .context("offhand requires an attached actor with n_buki_l")?;
        let data = if resident {
            EffectAssets::load(source, main)?
        } else {
            EffectAssets::load_owner(source, main)?
        };
        let owner = data.owner_path().context("offhand MDL absent")?.to_owned();
        if let Some((mdl, mounts)) = expected {
            ensure!(
                owner == mdl
                    && data
                        .mounts
                        .attachments
                        .iter()
                        .map(|m| &m.data.avfx_path)
                        .eq(mounts),
                "offhand manifest disagrees with loaded owner or mounts"
            );
        }
        let bind = super::pose::owner_skeleton(&data.mounts)
            .or(data.standalone_rig.as_ref())
            .context("offhand needs one owner skeleton")?
            .clone();
        let points = if let Some(m) = data.mounts.attachments.first() {
            ensure!(
                m.model_path == owner,
                "offhand mount belongs to another MDL"
            );
            m.data.bind_points.clone()
        } else {
            data.standalone_points
        };
        let ctx = self.renderer.context();
        let mesh = ctx.create_model_with_skeleton(&data.model, data.options, Some(&bind));
        ensure!(
            mesh.joint_count() > 0,
            "offhand MDL has no prepared joint palette"
        );
        let bind = OwnerPose::from_bind(bind);
        let mut next = Offhand {
            visible: true,
            bind_sphere: mesh.bounds_sphere(),
            mesh,
            rest: bind.rest_locals(),
            bind,
            points,
            elements: Vec::new(),
            grip: options.grip,
            mounts: Self::forward_depth_mounts(data.mounts),
            playback: None,
            origin: self.tick,
            last_hand: None,
            owner,
        };
        next.pose(ctx, hand)?;
        self.offhand = Some(next);
        Ok(())
    }

    pub fn clear_offhand(&mut self) {
        self.offhand = None;
    }
    /// Toggle the surface and its resident draws without changing the camera or playback time.
    pub fn set_offhand_visible(&mut self, visible: bool) -> Result<()> {
        let o = self.offhand.as_mut().context("no offhand equipped")?;
        o.visible = visible;
        o.mesh.set_surfaces_visible(visible);
        Ok(())
    }
    /// External trigger for this hand only, even if both weapons load the same MDL.
    pub fn trigger_offhand(&mut self, scheduler: i32, trigger: i32) -> Result<bool> {
        let Some(o) = &mut self.offhand else {
            return Ok(false);
        };
        let Some(p) = &mut o.playback else {
            return Ok(false);
        };
        p.trigger_attachment_document(&o.owner, scheduler, trigger)
            .map_err(anyhow::Error::msg)
    }
    pub fn offhand_elements(&self) -> Option<&[VfxBinderTargetSnapshot]> {
        self.offhand.as_ref().map(|o| o.elements.as_slice())
    }
    /// Publish the explicitly selected caster weapon's ElementIds to an action.
    /// With no separately installed offhand, the single loaded owner can itself
    /// be the selected secondary weapon (the weapon-only preview convention).
    pub fn refresh_action_weapon(&mut self, target: crate::timeline::BindTarget) -> Result<()> {
        ensure!(
            target.origin == 1 && matches!(target.kind, 1 | 2),
            "specify caster weapon/offhand binding"
        );
        let elements = if let Some(o) = self.offhand.as_ref().filter(|_| target.kind == 2) {
            o.elements.clone()
        } else if self.mounts.attachments.is_empty() && self.pose.is_some() {
            self.standalone_elements.clone()
        } else {
            let [mount] = self.mounts.attachments.as_slice() else {
                anyhow::bail!("live ElementIds require a single mounted weapon owner")
            };
            let sk = mount
                .data
                .skeleton
                .as_ref()
                .context("missing weapon skeleton")?;
            mount
                .data
                .binder_targets_for_pose(&assets::SkeletonPose::rest_pose(sk))
                .map_err(anyhow::Error::msg)?
        };
        self.action_player_mut()
            .context("no active action")?
            .set_caster_elements(target, elements)
    }
    /// Publish both equipped hands after posing, before the next C012 input.
    /// Leave explicit point overrides and an absent offhand under host control.
    pub fn refresh_equipped_action_weapons(&mut self) -> Result<()> {
        for kind in [1, 2] {
            let selector = crate::timeline::BindTarget {
                origin: 1,
                kind,
                id: -1,
            };
            if (kind == 1 || self.offhand.is_some())
                && self
                    .action
                    .as_ref()
                    .is_some_and(|(a, _)| a.has_binding(selector))
            {
                self.refresh_action_weapon(selector)?;
            }
        }
        Ok(())
    }
    pub(super) fn update_offhand_grip(&mut self, hand: Option<[f32; 16]>) -> Result<()> {
        if let Some(o) = &mut self.offhand {
            o.pose(
                self.renderer.context(),
                hand.context("offhand actor grip n_buki_l missing")?,
            )?;
        }
        Ok(())
    }
}

impl Offhand {
    fn pose(&mut self, ctx: &ModelRenderContext, hand: [f32; 16]) -> Result<()> {
        if self.last_hand == Some(hand) {
            return Ok(());
        }
        let trs = self.bind.gripped_locals(&hand, &self.grip, &self.rest)?;
        let (joints, pose) = self.bind.resolve(
            &trs,
            self.mesh.joint_names(),
            &mut self.mounts,
            self.playback.as_mut(),
        )?;
        self.elements =
            assets::VfxRuntime::bind_point_pose_targets(&self.points, &self.bind.bind, &pose)
                .map_err(anyhow::Error::msg)?;
        self.mesh.update_joint_matrices(ctx, &joints);
        let transform = Mat4::from_cols_array(&hand) * Mat4::from_cols_array(&self.grip);
        let center = transform.transform_point3(Vec3::from_array(self.bind_sphere.0));
        let (scale, _, _) = transform.to_scale_rotation_translation();
        self.mesh.set_bounds_sphere(
            center.to_array(),
            self.bind_sphere.1 * scale.abs().max_element(),
        );
        self.last_hand = Some(hand);
        Ok(())
    }
    pub(super) fn reset(&mut self) {
        self.restart_at(0);
    }
    pub(super) fn restart_at(&mut self, tick: u64) {
        self.playback = None;
        self.origin = tick;
    }
    pub(super) fn advance(
        &mut self,
        ctx: &ModelRenderContext,
        view: VfxCameraViewSnapshot,
        tick: u64,
        random: &super::particles::ParticleEnvironment,
    ) -> Result<()> {
        if self.mounts.attachments.is_empty() {
            return Ok(());
        }
        if self.playback.is_none() {
            self.playback = Some(
                ctx.create_weapon_vfx_particles_with_stream(
                    &self.mesh,
                    &self.mounts,
                    view,
                    Some(random.0.clone()),
                )
                .map_err(anyhow::Error::msg)?,
            );
        }
        let p = self.playback.as_mut().unwrap();
        p.set_camera_view(&self.mesh, view)
            .map_err(anyhow::Error::msg)?;
        p.advance_to((tick - self.origin) as f32 / 60.);
        Ok(())
    }
    pub(super) fn upload(&mut self, ctx: &ModelRenderContext, tick: u64) {
        if let Some(p) = &mut self.playback {
            p.update(ctx, &self.mesh, (tick - self.origin) as f32 / 60.);
        }
    }
    pub(super) fn diagnostics(&self) -> Vec<String> {
        let mut d = self.mounts.diagnostics.clone();
        if let Some(p) = &self.playback {
            d.extend(
                p.binding_diagnostics()
                    .into_iter()
                    .map(|(m, e)| format!("{m}: {e}")),
            );
            d.extend_from_slice(p.aura_diagnostics());
        }
        d.into_iter()
            .map(|s| format!("offhand {}: {s}", self.owner))
            .collect()
    }
}
