//! Resident and action use independent resource tables and clocks. The scene
//! merges their document draw ranges before selecting soft/decal pass segments.
use super::*;
use assets::{VfxCameraViewSnapshot, VfxMeshInstance, VfxQuad};
use timeline::{ActionPlayer, DrawAssets};

#[derive(Default)]
pub(super) struct Composite {
    pub(super) batch: Option<scene::VfxParticles>,
    quads: Vec<VfxQuad>,
    meshes: Vec<VfxMeshInstance>,
    quad_tone: Vec<bool>,
    mesh_tone: Vec<bool>,
    quad_color: Vec<[f32; 4]>,
    mesh_color: Vec<[f32; 4]>,
}

impl EffectPlayer {
    #[cfg(target_arch = "wasm32")]
    pub(super) fn set_mounts(&mut self, mounts: WeaponVfxAttachments) {
        self.pose = pose::OwnerPose::new(&mounts);
        self.mounts = Self::forward_depth_mounts(mounts);
        self.resident_origin = self.tick;
        self.particles = None;
    }

    /// Start an action at its local time zero without restarting the resident
    /// Documents or the host pose clock.
    pub fn play_action(&mut self, mut action: ActionPlayer, draw: DrawAssets) {
        self.clear_action();
        action.environment = self.random.borrowed();
        action.reset();
        self.composite.batch = Some(
            self.renderer
                .context()
                .create_vfx_particles(&draw.textures, &draw.models),
        );
        self.action = Some((action, self.tick));
    }

    pub fn clear_action(&mut self) {
        self.action = None;
        self.composite = Default::default();
    }
    pub fn action_player_mut(&mut self) -> Option<&mut ActionPlayer> {
        self.action.as_mut().map(|(p, _)| p)
    }
    /// Deterministically seek the active action (and its C012 AVFX particles)
    /// to `seconds` at 60 Hz simulation cadence without intermediate GPU draws.
    pub fn seek_action_with_host(
        &mut self,
        seconds: f64,
        mut host: impl FnMut(&mut Self, f64) -> Result<()>,
    ) -> Result<()> {
        anyhow::ensure!(
            seconds.is_finite() && seconds >= 0.,
            "invalid action seek time"
        );
        let Some((_, origin)) = self.action else {
            return Ok(());
        };
        let target_tick = (seconds * 60.).round() as u64;
        let current_tick = self.tick.saturating_sub(origin);
        let start_step = if target_tick < current_tick {
            self.random.reset();
            if let Some((action, orig)) = &mut self.action {
                action.environment = self.random.borrowed();
                action.reset();
                *orig = self.tick;
            }
            0
        } else {
            current_tick
        };
        let origin = self.action.as_ref().unwrap().1;
        for step in start_step..=target_tick {
            self.tick = origin + step;
            let t = step as f64 / 60.;
            host(self, t)?;
            if let Some((action, _)) = &mut self.action {
                action.advance_to(t)?;
            }
        }
        self.remainder = 0.;
        Ok(())
    }
    pub fn trigger_resident(
        &mut self,
        owner_mdl: &str,
        scheduler: i32,
        trigger: i32,
    ) -> Result<bool> {
        let Some(p) = &mut self.particles else {
            return Ok(false);
        };
        p.trigger_attachment_document(owner_mdl, scheduler, trigger)
            .map_err(anyhow::Error::msg)
    }

    /// Replace the ordered target provider for occurrences using this second endpoint.
    pub fn set_action_target(&mut self, binding: timeline::Binding) -> Result<()> {
        let (action, _) = self
            .action
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no active action"))?;
        action.set_target_binding(binding)
    }

    pub(super) fn prepare_playback(&mut self, view: VfxCameraViewSnapshot) -> Result<()> {
        if let Some((action, _)) = &mut self.action {
            action.set_camera_view(view)?;
        }
        if self.mounts.attachments.is_empty() {
            return Ok(());
        }
        if self.particles.is_none() {
            let (ctx, weapon) = self.renderer.scene_parts();
            self.particles = Some(
                ctx.create_weapon_vfx_particles_with_stream(
                    weapon,
                    &self.mounts,
                    view,
                    Some(self.random.0.clone()),
                )
                .map_err(anyhow::Error::msg)?,
            );
        }
        self.renderer
            .set_weapon_vfx_camera_view(self.particles.as_mut().unwrap(), view)
            .map_err(anyhow::Error::msg)
    }

    pub(super) fn advance_documents(&mut self) -> Result<()> {
        if let Some(p) = &mut self.particles {
            p.advance_to((self.tick - self.resident_origin) as f32 / 60.);
        }
        if let Some((a, origin)) = &mut self.action {
            a.advance_to((self.tick - *origin) as f64 / 60.)?;
        }
        Ok(())
    }

    pub(super) fn upload_documents(&mut self, t: f32) {
        if let Some(p) = &mut self.particles {
            self.renderer.update_weapon_vfx_particles(p, t);
        }
        let c = &mut self.composite;
        let (Some((action, _)), Some(batch)) = (&self.action, &mut c.batch) else {
            return;
        };
        let capacity = batch.capacity();
        c.quads.clear();
        c.meshes.clear();
        c.quad_tone.clear();
        c.mesh_tone.clear();
        c.quad_color.clear();
        c.mesh_color.clear();
        batch.clear_document_ranges();
        let mut order_base = 0u64;
        let mut particle_base = 0;
        action.visit_packets(
            |position, sko, serial, definitions, qs, ms, qt, mt, color| {
                let max_order = qs
                    .iter()
                    .filter_map(|q| q.draw_order)
                    .chain(ms.iter().filter_map(|m| m.draw_order))
                    .max();
                let q_count = qs.len().min(capacity - c.quads.len());
                let m_count = ms.len().min(capacity - c.meshes.len());
                c.quad_tone.extend_from_slice(&qt[..q_count]);
                c.mesh_tone.extend_from_slice(&mt[..m_count]);
                c.quad_color.extend(std::iter::repeat_n(color, q_count));
                c.mesh_color.extend(std::iter::repeat_n(color, m_count));
                c.quads.extend(qs[..q_count].iter().copied().map(|mut q| {
                    q.draw_order = q.draw_order.map(|order| order.saturating_add(order_base));
                    q.particle_index += particle_base;
                    q
                }));
                c.meshes.extend(ms[..m_count].iter().copied().map(|mut m| {
                    m.draw_order = m.draw_order.map(|order| order.saturating_add(order_base));
                    m
                }));
                if let Some(last) = max_order {
                    let end = order_base.saturating_add(last).saturating_add(1);
                    batch.register_document_range(
                        order_base..end,
                        position,
                        sko,
                        (self.mounts.attachments.len() + serial + 1) as u32,
                    );
                    // The gap stops a consecutive GPU draw run from merging two Documents.
                    order_base = end.saturating_add(1);
                }
                particle_base += definitions;
            },
        );
        batch.update_documents(
            self.renderer.context(),
            &c.quads,
            &c.quad_tone,
            &c.quad_color,
        );
        batch.update_mesh_documents(
            self.renderer.context(),
            &c.meshes,
            &c.mesh_tone,
            &c.mesh_color,
        );
    }
}
