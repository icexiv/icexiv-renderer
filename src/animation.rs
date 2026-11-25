//! Playback of explicitly selected PAP clips on their matching SKLB.
use crate::{EquipmentModel, ModelError};
use glam::{Mat3, Mat4, Quat, Vec3};
use icexiv_data::{havok, mdl, pap::Pap, sklb::Sklb};
use std::collections::HashMap;

// The c0101 resident/action.pap contains 162 clips and >2M borrowed spline
// bytes. Each byte counts as a value. Keep the independent allocation cap.
pub(crate) fn parse_pap(bytes: &[u8]) -> havok::Result<Pap<'_>> {
    Pap::parse_with_limits(
        bytes,
        havok::Limits {
            max_values: 8 << 20,
            ..Default::default()
        },
    )
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationError {
    #[error(transparent)]
    Skeleton(#[from] icexiv_data::sklb::Error),
    #[error(transparent)]
    Clip(#[from] havok::Error),
    #[error(transparent)]
    Mesh(#[from] mdl::Error),
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("cannot animate equipment: {0}")]
    Binding(&'static str),
    #[error("weighted model bone missing from skeleton: {0}")]
    MissingBone(String),
}
use AnimationError::Binding;

// Spline knots borrow the PAP buffer. Keep that buffer, not the whole tag graph,
// so frames sample the decoded clip without reparsing the asset or baking it.
struct Clip<'a> {
    spline: havok::SplineAnimation<'a>,
    tracks: Vec<Option<usize>>,
    additive: bool,
}
#[derive(Clone, Copy)]
enum RecordRef<'a> {
    Index(usize),
    NormalName(&'a [u8]),
}
self_cell::self_cell! {
    struct OwnedClip {
        owner: Vec<u8>,
        #[covariant]
        dependent: Clip,
    }
}

pub struct PapPose {
    clip: OwnedClip,
    overlay: Option<(OwnedClip, f32)>,
    bones: HashMap<Vec<u8>, usize>,
    parents: Vec<Option<usize>>,
    order: Vec<usize>,
    local_bind: Vec<havok::Sample>,
    locals: Vec<havok::Sample>,
    world: Vec<Mat4>,
    inverse_bind: Vec<Mat4>,
}

fn trs(t: Vec3, q: [f32; 4], s: Vec3) -> Result<Mat4, AnimationError> {
    let q = Quat::from_array(q);
    if !q.length_squared().is_finite() || q.length_squared() < f32::MIN_POSITIVE {
        return Err(Binding("invalid joint rotation"));
    }
    Ok(Mat4::from_scale_rotation_translation(s, q.normalize(), t))
}

fn accumulate(
    world: &mut [Mat4],
    order: &[usize],
    parents: &[Option<usize>],
) -> Result<(), AnimationError> {
    for &i in order {
        if let Some(parent) = parents[i] {
            world[i] = world[parent] * world[i];
        }
        if !world[i].is_finite() {
            return Err(Binding("non-finite joint transform"));
        }
    }
    Ok(())
}

impl PapPose {
    /// C010 identifies a PAP record by name, not by file path or Havok index.
    pub fn named(sklb: &[u8], bytes: Vec<u8>, name: &[u8]) -> Result<Self, AnimationError> {
        Self::load(sklb, bytes, RecordRef::NormalName(name), None)
    }
    /// `record` indexes the PAP record list, not the Havok object stream.
    pub fn new(sklb: &[u8], pap: Vec<u8>, record: usize) -> Result<Self, AnimationError> {
        Self::load(sklb, pap, RecordRef::Index(record), None)
    }

    /// Sample a normal base and a weighted additive clip on a common seconds
    /// clock. Each clip independently holds its final frame after its duration.
    pub fn with_additive(
        sklb: &[u8],
        pap: Vec<u8>,
        record: usize,
        overlay: Vec<u8>,
        overlay_record: usize,
        weight: f32,
    ) -> Result<Self, AnimationError> {
        if !(0. ..=1.).contains(&weight) {
            return Err(Binding("layer weight must be in [0,1]"));
        }
        Self::load(
            sklb,
            pap,
            RecordRef::Index(record),
            Some((overlay, overlay_record, weight)),
        )
    }

    fn load(
        sklb: &[u8],
        pap: Vec<u8>,
        record: RecordRef<'_>,
        overlay: Option<(Vec<u8>, usize, f32)>,
    ) -> Result<Self, AnimationError> {
        let sk = Sklb::parse(sklb)?;
        let ids = sk.tagfile.animation_skeleton_ids(1 << 20)?;
        let [id] = ids.as_slice() else {
            return Err(Binding("expected one container skeleton"));
        };
        let skeleton = sk
            .skeletons(8 << 20)?
            .into_iter()
            .find(|s| s.object_id == *id)
            .ok_or(Binding("container skeleton is absent"))?;
        let n = skeleton.bones.len();
        let decode = |pap, record| {
            OwnedClip::try_new(pap, |bytes| -> Result<_, AnimationError> {
                let pap = parse_pap(bytes)?;
                let selected = match record {
                    RecordRef::Index(i) => {
                        pap.records().nth(i).ok_or(Binding("unknown PAP record"))?
                    }
                    RecordRef::NormalName(name) => {
                        let mut records = pap.records().filter(|r| r.name == name);
                        let first = records
                            .next()
                            .ok_or(Binding("C010 name missing from supplied PAP"))?;
                        if records.next().is_some() {
                            return Err(Binding("ambiguous C010 name in supplied PAP"));
                        }
                        first
                    }
                };
                let binding = pap.binding(selected.havok_index)?;
                if matches!(record, RecordRef::NormalName(_)) && binding.blend_hint != 0 {
                    return Err(Binding("C010 pose layer requires a normal PAP record"));
                }
                if !matches!(binding.blend_hint, 0 | 1) {
                    return Err(Binding("unsupported blend hint"));
                }
                if let (Some(a), Some(b)) = (binding.original_skeleton_name, skeleton.name)
                    && !a.is_empty()
                    && a != b
                    && !skeleton.bones.iter().any(|bone| {
                        bone.parent.is_none() && Some(bone.name) == a.rsplit(|&c| c == b':').next()
                    })
                {
                    return Err(Binding("PAP and SKLB skeleton names differ"));
                }
                let spline = pap
                    .tagfile
                    .as_ref()
                    .ok_or(Binding("missing PAP graph"))?
                    .spline_animation(binding.animation_object_id, Default::default())?;
                let map = binding.transform_track_to_bone;
                if spline.blocks.first().map(|b| b.track_count()) != Some(map.len()) {
                    return Err(Binding("explicit transform track map required"));
                }
                let mut used = vec![false; n];
                let mut tracks = Vec::with_capacity(map.len());
                for &target in map {
                    if target == -1 {
                        tracks.push(None);
                        continue;
                    }
                    let i = target as usize;
                    let used = used
                        .get_mut(i)
                        .ok_or(Binding("track outside selected skeleton"))?;
                    if std::mem::replace(used, true) {
                        return Err(Binding("duplicate track destination"));
                    }
                    tracks.push(Some(i));
                }
                Ok(Clip {
                    spline,
                    tracks,
                    additive: binding.blend_hint == 1,
                })
            })
        };
        let clip = decode(pap, record)?;
        let overlay = match overlay {
            Some((pap, record, weight)) => {
                let layer = decode(pap, RecordRef::Index(record))?;
                if clip.borrow_dependent().additive || !layer.borrow_dependent().additive {
                    return Err(Binding(
                        "layered pose requires a normal base then an additive clip",
                    ));
                }
                Some((layer, weight))
            }
            None => None,
        };
        let mut bones = HashMap::new();
        let mut local_bind = Vec::with_capacity(n);
        let mut parents = Vec::with_capacity(n);
        let mut children = vec![Vec::new(); n];
        let mut order = Vec::with_capacity(n);
        for (i, bone) in skeleton.bones.iter().enumerate() {
            if bones.insert(bone.name.to_vec(), i).is_some() {
                return Err(Binding("ambiguous skeleton bone name"));
            }
            parents.push(bone.parent);
            match bone.parent {
                Some(parent) => children[parent].push(i),
                None => order.push(i),
            }
            let t = bone.rest;
            local_bind.push(havok::Sample {
                translation: Vec3::from_slice(&t.translation).to_array(),
                rotation: t.rotation,
                scale: Vec3::from_slice(&t.scale).to_array(),
            });
        }
        let mut at = 0;
        while at < order.len() {
            order.extend_from_slice(&children[order[at]]);
            at += 1;
        }
        let mut world = local_bind
            .iter()
            .map(|s| trs(s.translation.into(), s.rotation, s.scale.into()))
            .collect::<Result<Vec<_>, _>>()?;
        accumulate(&mut world, &order, &parents)?;
        let inverse_bind: Vec<_> = world.iter().map(|m| m.inverse()).collect();
        if inverse_bind.iter().any(|m| !m.is_finite()) {
            return Err(Binding("singular bind transform"));
        }
        let mut pose = Self {
            clip,
            overlay,
            bones,
            parents,
            order,
            locals: local_bind.clone(),
            local_bind,
            world,
            inverse_bind,
        };
        pose.sample(0.)?;
        Ok(pose)
    }

    pub fn duration_seconds(&self) -> f64 {
        let base = self.clip.borrow_dependent().spline.duration_seconds;
        base.max(
            self.overlay
                .as_ref()
                .map_or(0., |(c, _)| c.borrow_dependent().spline.duration_seconds),
        )
        .into()
    }

    /// Seek absolutely in seconds. Times outside the clip clamp to its endpoints.
    pub fn sample(&mut self, seconds: f64) -> Result<(), AnimationError> {
        self.locals.copy_from_slice(&self.local_bind);
        for (clip, weight) in
            std::iter::once((&self.clip, 1.)).chain(self.overlay.iter().map(|(c, w)| (c, *w)))
        {
            let clip = clip.borrow_dependent();
            let samples = clip.spline.sample(seconds * 1000., 8 << 20, 8 << 20)?;
            if samples.len() != clip.tracks.len() {
                return Err(Binding("block track count differs"));
            }
            for (sample, &target) in samples.into_iter().zip(&clip.tracks) {
                if let Some(i) = target {
                    self.locals[i] = if clip.additive {
                        blend_additive(self.locals[i], sample, weight)
                    } else {
                        sample
                    };
                }
            }
        }
        self.update_world()
    }

    fn update_world(&mut self) -> Result<(), AnimationError> {
        for (m, s) in self.world.iter_mut().zip(&self.locals) {
            *m = trs(s.translation.into(), s.rotation, s.scale.into())?;
        }
        accumulate(&mut self.world, &self.order, &self.parents)
    }

    /// Crossfade two evaluated local poses (each may already contain additive
    /// motion). A subtree selects all descendants of one bone. The next seek
    /// rebuilds the pose, so seeking/rewinding doesn't accumulate blends.
    pub fn blend_current(
        &mut self,
        other: &Self,
        weight: f32,
        subtree: Option<&[u8]>,
    ) -> Result<(), AnimationError> {
        if !(0. ..=1.).contains(&weight) {
            return Err(Binding("blend weight outside [0,1]"));
        }
        if self.bones != other.bones
            || self.parents != other.parents
            || self.local_bind != other.local_bind
        {
            return Err(Binding("cannot blend different skeletons"));
        }
        let root = subtree
            .map(|name| {
                self.bones
                    .get(name)
                    .copied()
                    .ok_or(Binding("blend root not found"))
            })
            .transpose()?;
        if weight == 0. {
            return Ok(());
        }
        let mut active = vec![root.is_none(); self.locals.len()];
        for &i in &self.order {
            if root == Some(i) || self.parents[i].is_some_and(|p| active[p]) {
                active[i] = true
            }
            if active[i] {
                self.locals[i] = blend_local(self.locals[i], other.locals[i], weight);
            }
        }
        self.update_world()
    }

    /// Current local-to-skeleton matrix in game asset units, before camera fitting.
    pub fn bone_world(&self, name: &[u8]) -> Option<Mat4> {
        self.bones.get(name).map(|&i| self.world[i])
    }
    pub fn rest_bone_world(&self, name: &[u8]) -> Option<Mat4> {
        self.bones
            .get(name)
            .map(|&i| self.inverse_bind[i].inverse())
    }
    /// Parent-local T3, XYZW quaternion and S3. The caller selects the order
    /// by bone name, so two unrelated skeletons cannot alias by integer index.
    pub fn local_pose<'a>(
        &self,
        names: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<Vec<f32>, AnimationError> {
        let mut trs = Vec::new();
        for name in names {
            let i = *self
                .bones
                .get(name)
                .ok_or(Binding("local pose bone is absent"))?;
            let s = &self.locals[i];
            trs.extend(s.translation);
            trs.extend(s.rotation);
            trs.extend(s.scale);
        }
        Ok(trs)
    }
}

fn blend_local(a: havok::Sample, b: havok::Sample, t: f32) -> havok::Sample {
    if t == 0. {
        return a;
    }
    if t == 1. {
        return b;
    }
    havok::Sample {
        translation: Vec3::from(a.translation)
            .lerp(b.translation.into(), t)
            .into(),
        rotation: Quat::from_array(a.rotation)
            .normalize()
            .slerp(Quat::from_array(b.rotation).normalize(), t)
            .to_array(),
        scale: Vec3::from(a.scale).lerp(b.scale.into(), t).into(),
    }
}

/// Havok doBlendAdditive4: interpolate from base to delta * base in local
/// QsTransform space. Scale composes separately from rotation and translation.
fn blend_additive(base: havok::Sample, delta: havok::Sample, w: f32) -> havok::Sample {
    let t = Vec3::from(base.translation);
    let s = Vec3::from(base.scale);
    let q = Quat::from_array(base.rotation).normalize();
    let d = Quat::from_array(delta.rotation);
    havok::Sample {
        translation: t.lerp(d * t + Vec3::from(delta.translation), w).to_array(),
        scale: s.lerp(s * Vec3::from(delta.scale), w).to_array(),
        rotation: q.lerp(d * q, w).to_array(),
    }
}

pub struct AnimatedEquipment {
    pub(crate) model: EquipmentModel,
    pose: PapPose,
    skin: Vec<Option<mdl::SkinInfluences>>,
    pub(crate) vertices: Vec<[f32; 8]>,
}

impl AnimatedEquipment {
    pub fn new(bytes: &[u8], pose: PapPose) -> Result<Self, AnimationError> {
        let model = EquipmentModel::from_mdl(bytes)?;
        let mdl = mdl::Mdl::parse(bytes)?;
        let mut skin = Vec::with_capacity(model.vertices.len());
        let mut weighted = false;
        for &mesh in &model.meshes {
            let g = mdl.geometry(0, mesh)?;
            let meta = g.metadata();
            let mut palette = Vec::new();
            if meta.bone_table_index != u16::MAX && mdl.bone_table_count() != 0 {
                for bone in mdl.bone_table(meta.bone_table_index.into())? {
                    let name = mdl.bone_name(bone.into())?;
                    palette.push((name, pose.bones.get(name).copied()));
                }
            }
            for v in 0..usize::from(meta.vertex_count) {
                let mut influence = g.skin_influences(v)?;
                if let Some(s) = &mut influence {
                    for j in 0..usize::from(s.count) {
                        if s.weights[j] == 0. {
                            continue;
                        }
                        let &(name, target) = palette
                            .get(usize::from(s.indices[j]))
                            .ok_or(Binding("weighted joint outside mesh palette"))?;
                        s.indices[j] = target
                            .ok_or_else(|| {
                                AnimationError::MissingBone(
                                    String::from_utf8_lossy(name).into_owned(),
                                )
                            })?
                            .try_into()
                            .map_err(|_| Binding("joint index overflow"))?;
                        weighted = true;
                    }
                }
                skin.push(influence);
            }
        }
        if !weighted {
            return Err(Binding("model has no weighted vertices"));
        }
        let vertices = model.vertices.clone();
        let mut animated = Self {
            model,
            pose,
            skin,
            vertices,
        };
        animated.deform()?;
        Ok(animated)
    }
    pub fn pose(&self) -> &PapPose {
        &self.pose
    }

    /// Fitted position, normal and UV for each rendered vertex.
    pub fn sample(&mut self, seconds: f64) -> Result<&[[f32; 8]], AnimationError> {
        self.pose.sample(seconds)?;
        self.deform()?;
        Ok(&self.vertices)
    }

    fn deform(&mut self) -> Result<(), AnimationError> {
        let joints: Vec<_> = self
            .pose
            .world
            .iter()
            .zip(&self.pose.inverse_bind)
            .map(|(w, inv)| *w * *inv)
            .collect();
        for ((out, rest), s) in self
            .vertices
            .iter_mut()
            .zip(&self.model.vertices)
            .zip(&self.skin)
        {
            if let Some(s) = s {
                *out = skin_vertex(rest, s, &joints, self.model.center, self.model.radius)?;
            }
        }
        Ok(())
    }
}

fn skin_vertex(
    rest: &[f32; 8],
    s: &mdl::SkinInfluences,
    joints: &[Mat4],
    center: Vec3,
    radius: f32,
) -> Result<[f32; 8], AnimationError> {
    let mut m = Mat4::ZERO;
    let mut sum = 0.;
    for j in 0..usize::from(s.count) {
        let w = s.weights[j];
        if w != 0. {
            m += joints[usize::from(s.indices[j])] * w;
            sum += w;
        }
    }
    if sum == 0. {
        return Ok(*rest);
    }
    m *= 1. / sum;
    let p = (m.transform_point3(Vec3::from_slice(rest) * radius + center) - center) / radius;
    let linear = Mat3::from_mat4(m);
    let n = if linear.determinant().abs() > f32::MIN_POSITIVE {
        (linear.inverse().transpose() * Vec3::from_slice(&rest[3..])).normalize_or_zero()
    } else {
        Vec3::ZERO
    }; // shader derives a geometric normal when a joint collapses
    if !p.is_finite() || !n.is_finite() {
        return Err(Binding("non-finite skinned vertex"));
    }
    Ok([p.x, p.y, p.z, n.x, n.y, n.z, rest[6], rest[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_interpolates_after_composition_in_joint_local_space() {
        let base = havok::Sample {
            translation: Vec3::X.to_array(),
            rotation: Quat::IDENTITY.to_array(),
            scale: [2.; 3],
        };
        let delta = havok::Sample {
            translation: Vec3::Z.to_array(),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array(),
            scale: [3.; 3],
        };
        let half = blend_additive(base, delta, 0.5);
        assert!(Vec3::from(half.translation).abs_diff_eq(Vec3::splat(0.5), 1e-6));
        assert!(
            Quat::from_array(half.rotation)
                .abs_diff_eq(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4), 1e-6)
        );
        assert_eq!(half.scale, [4.; 3]);
    }

    #[test]
    fn parent_motion_and_secondary_weights_act_in_model_units() {
        // Child precedes parent in the file; its bind pivot is at +2 on X.
        let bind = Mat4::from_translation(2. * Vec3::X);
        let mut world = [bind, Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)];
        accumulate(&mut world, &[1, 0], &[Some(1), None]).unwrap();
        let joints = [world[0] * bind.inverse(), Mat4::IDENTITY];
        // Vertex at (4,0,0) is fitted about (2,0,0) with radius 2.
        // Half the weight lies in lane 7. Unused lanes need not be valid indices.
        let mut s = mdl::SkinInfluences {
            count: 8,
            weights: [0.; 8],
            indices: [65535; 8],
        };
        s.weights[1] = 0.4;
        s.weights[7] = 0.4;
        s.indices[1] = 0;
        s.indices[7] = 1;
        let result = skin_vertex(
            &[1., 0., 0., 1., 0., 0., 0.25, 0.75],
            &s,
            &joints,
            2. * Vec3::X,
            2.,
        )
        .unwrap();
        assert!(Vec3::from_slice(&result).abs_diff_eq(Vec3::Y, 1e-6));
        assert!(Vec3::from_slice(&result[3..]).abs_diff_eq((Vec3::X + Vec3::Y).normalize(), 1e-6));
        assert_eq!(&result[6..], &[0.25, 0.75]);
    }

    /// Use a matching weighted MDL, base SKLB and PAP containing a moving normal clip.
    #[test]
    #[ignore = "requires extracted assets: MODEL_MDL, BASE_SKLB, CLIP_PAP, PAP_RECORD"]
    fn seek_real_clip_on_equipment() {
        let bytes = |key| std::fs::read(std::env::var(key).unwrap()).unwrap();
        let index = std::env::var("PAP_RECORD").unwrap().parse().unwrap();
        let pose = PapPose::new(&bytes("BASE_SKLB"), bytes("CLIP_PAP"), index).unwrap();
        let duration = pose.duration_seconds();
        let mut model = AnimatedEquipment::new(&bytes("MODEL_MDL"), pose).unwrap();
        let a = model.sample(0.).unwrap().to_vec();
        let b = model.sample(duration * 0.37).unwrap();
        assert!(
            a.iter()
                .zip(b)
                .any(|(a, b)| (Vec3::from_slice(a) - Vec3::from_slice(b)).length() > 1e-4)
        );
        assert_eq!(a, model.sample(0.).unwrap());
    }

    #[test]
    #[ignore = "requires actor SKLB and a moving normal CLIP_PAP/PAP_RECORD"]
    fn crossfade_updates_only_requested_subtree() {
        let sk = std::fs::read(std::env::var("BASE_SKLB").unwrap()).unwrap();
        let pap = std::fs::read(std::env::var("CLIP_PAP").unwrap()).unwrap();
        let record = std::env::var("PAP_RECORD").unwrap().parse().unwrap();
        let mut base = PapPose::new(&sk, pap.clone(), record).unwrap();
        let mut next = PapPose::new(&sk, pap, record).unwrap();
        next.sample(next.duration_seconds() * 0.37).unwrap();
        let hand = b"j_te_r".as_slice();
        let reference = next.local_pose([hand]).unwrap();
        let before = base.local_pose([hand]).unwrap();
        assert_ne!(before, reference);
        let left = base.bone_world(b"n_buki_l").unwrap();
        base.blend_current(&next, 1., Some(hand)).unwrap();
        assert_eq!(base.local_pose([hand]).unwrap(), reference);
        assert_eq!(base.bone_world(b"n_buki_l").unwrap(), left);
        base.sample(0.).unwrap();
        assert_eq!(base.local_pose([hand]).unwrap(), before);
        base.blend_current(&next, 0.5, None).unwrap();
        let blended = base.local_pose([hand]).unwrap();
        assert_ne!(blended, before);
        assert_ne!(blended, reference);
    }
}
