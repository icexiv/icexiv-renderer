use super::*;
use anyhow::{Context, ensure};
use assets::{BoneTransform, ModelSkeleton, SkeletonInverseBindCache, SkeletonPose};

// Do not reuse a joint buffer across unrelated owner skeletons.
pub(super) fn mdl_points(bytes: &[u8]) -> Result<Vec<assets::avfx::VfxBindPoint>> {
    Ok(assets::mdl_element_ids_from_mdl_bytes(bytes)?
        .into_iter()
        .map(|p| assets::avfx::VfxBindPoint {
            id: p.id,
            parent_bone: p.parent_bone,
            translate: p.translate,
            rotate: p.rotate,
        })
        .collect())
}

pub(super) fn owner_skeleton(mounts: &WeaponVfxAttachments) -> Option<&ModelSkeleton> {
    let [mount] = mounts.attachments.as_slice() else {
        return None;
    };
    mount.data.skeleton.as_ref()
}

pub(super) struct OwnerPose {
    pub(super) bind: ModelSkeleton,
    inverse: SkeletonInverseBindCache,
}

impl OwnerPose {
    pub(super) fn new(mounts: &WeaponVfxAttachments) -> Option<Self> {
        Some(Self::from_bind(owner_skeleton(mounts)?.clone()))
    }
    pub(super) fn from_bind(bind: ModelSkeleton) -> Self {
        Self {
            bind,
            inverse: Default::default(),
        }
    }
    pub(super) fn rest_locals(&self) -> Vec<f32> {
        self.bind
            .rest_pose
            .iter()
            .flat_map(|t| t.translation.into_iter().chain(t.rotation).chain(t.scale))
            .collect()
    }
}

impl EffectPlayer {
    /// Attach all root bones to a hand frame. Matrices are column-major;
    /// `grip` maps weapon model space into the hand's local space.
    pub fn set_weapon_grip(&mut self, hand: &[f32; 16], grip: &[f32; 16]) -> Result<()> {
        self.set_posed_weapon_grip(hand, grip, &self.rest_local_pose())
    }

    /// As `set_weapon_grip`, preserving a weapon's own articulated local
    /// animation (e.g. moving book pages). Only its root(s) change space.
    pub fn set_posed_weapon_grip(
        &mut self,
        hand: &[f32; 16],
        grip: &[f32; 16],
        weapon_locals: &[f32],
    ) -> Result<()> {
        let host = self
            .pose
            .as_ref()
            .context("no unique weapon owner skeleton")?;
        self.set_local_pose(&host.gripped_locals(hand, grip, weapon_locals)?)
    }
}

impl OwnerPose {
    pub(super) fn gripped_locals(
        &self,
        hand: &[f32; 16],
        grip: &[f32; 16],
        weapon_locals: &[f32],
    ) -> Result<Vec<f32>> {
        use glam::{Mat4, Quat, Vec3};
        let transform = Mat4::from_cols_array(hand) * Mat4::from_cols_array(grip);
        ensure!(
            weapon_locals.len() == self.bind.bone_count() * 10,
            "wrong weapon pose length"
        );
        let mut trs = weapon_locals.to_vec();
        for (i, row) in trs.chunks_exact_mut(10).enumerate() {
            if self.bind.parent_indices[i] >= 0 {
                continue;
            }
            let q = Quat::from_array(row[3..7].try_into().unwrap());
            ensure!(
                q.length_squared().is_finite() && q.length_squared() > f32::EPSILON,
                "invalid weapon root quaternion"
            );
            let m = transform
                * Mat4::from_scale_rotation_translation(
                    Vec3::from_slice(&row[7..]),
                    q.normalize(),
                    Vec3::from_slice(&row[..3]),
                );
            ensure!(
                m.is_finite() && m.determinant().abs() > f32::MIN_POSITIVE,
                "singular hand transform"
            );
            let (scale, rotation, position) = m.to_scale_rotation_translation();
            ensure!(
                Mat4::from_scale_rotation_translation(scale, rotation, position)
                    .abs_diff_eq(m, 2e-4),
                "weapon root cannot represent shear or perspective"
            );
            row[..3].copy_from_slice(&position.to_array());
            row[3..7].copy_from_slice(&rotation.to_array());
            row[7..].copy_from_slice(&scale.to_array());
        }
        Ok(trs)
    }
}

impl EffectPlayer {
    /// Bone order for `set_local_pose`. Each record is local T, quaternion XYZW, S.
    pub fn bone_names(&self) -> &[String] {
        self.pose
            .as_ref()
            .map_or(&[], |p| p.bind.bone_names.as_slice())
    }

    pub fn rest_local_pose(&self) -> Vec<f32> {
        self.pose
            .as_ref()
            .map_or_else(Vec::new, OwnerPose::rest_locals)
    }

    /// Set one complete pose, including while paused or before the first birth.
    /// Restarting the particle clock preserves the owner's current pose.
    pub fn set_local_pose(&mut self, trs: &[f32]) -> Result<()> {
        let host = self
            .pose
            .as_mut()
            .context("no unique weapon owner skeleton")?;
        let (matrices, pose) = host.resolve(
            trs,
            self.renderer.joint_names(),
            &mut self.mounts,
            self.particles.as_mut(),
        )?;
        if self.mounts.attachments.is_empty() {
            self.standalone_elements = assets::VfxRuntime::bind_point_pose_targets(
                &self.standalone_points,
                &host.bind,
                &pose,
            )
            .map_err(anyhow::Error::msg)?;
        }
        self.renderer.update_joint_matrices(&matrices);
        Ok(())
    }
}

impl OwnerPose {
    pub(super) fn resolve(
        &mut self,
        trs: &[f32],
        joints: &[String],
        mounts: &mut WeaponVfxAttachments,
        particles: Option<&mut WeaponVfxParticles>,
    ) -> Result<(Vec<[f32; 16]>, SkeletonPose)> {
        ensure!(
            trs.len() == self.bind.bone_count() * 10,
            "wrong weapon pose length"
        );
        let mut pose = SkeletonPose::new(self.bind.bone_count());
        for (index, row) in trs.chunks_exact(10).enumerate() {
            ensure!(row.iter().all(|v| v.is_finite()), "nonfinite weapon pose");
            let norm: f32 = row[3..7].iter().map(|v| v * v).sum();
            ensure!(
                norm.is_finite() && norm > f32::EPSILON,
                "invalid weapon quaternion"
            );
            pose.set_transform(
                index,
                BoneTransform {
                    translation: row[..3].try_into().unwrap(),
                    rotation: row[3..7].try_into().unwrap(),
                    scale: row[7..].try_into().unwrap(),
                },
            )?;
        }
        let matrices = self.inverse.joint_matrices(&self.bind, &pose, joints);
        ensure!(
            matrices.iter().flatten().all(|v| v.is_finite()),
            "weapon joint overflow"
        );
        let Some(mount) = mounts.attachments.first_mut() else {
            return Ok((matrices, pose));
        };
        if let Some(p) = particles {
            p.set_attachment_pose(&mount.model_path, &pose)
                .map_err(anyhow::Error::msg)?;
        } else {
            mount
                .data
                .binder_targets_for_pose(&pose)
                .map_err(anyhow::Error::msg)?;
        }
        // Scene constructs birth targets from the playback copy's initial
        // locals. The immutable inverse-bind skeleton stays separate above.
        for (i, local) in mount
            .data
            .skeleton
            .as_mut()
            .unwrap()
            .rest_pose
            .iter_mut()
            .enumerate()
        {
            *local = pose.transform(i).unwrap();
        }
        Ok((matrices, pose))
    }
}
