//! Per-SKLB joint namespaces and partial animation layers. No partial layer can
//! change the base skeleton, weapon grip or offhand clock.
use super::{Actor, EffectPlayer};
use anyhow::{Context, Result, ensure};
use assets::{
    BoneTransform, ModelSkeleton, SkeletonPose,
    formats::havok::{spline::SplineAnimation, tagfile::Tagfile},
};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashSet;

pub(super) fn key(path: &str, index: usize) -> String {
    format!("\0{path}#{index}")
}

pub(super) struct PartialRig {
    pub path: String,
    pub skeleton: ModelSkeleton,
    pub anchors: Vec<usize>,
    inverse: Vec<Mat4>,
    delta: Vec<Mat4>,
    playback: Option<(PartialClip, u64, bool)>,
}
impl PartialRig {
    pub(super) fn new(
        path: String,
        skeleton: ModelSkeleton,
        base: &ModelSkeleton,
        connection: Option<usize>,
    ) -> Result<Self> {
        let n = skeleton.bone_count();
        ensure!(
            skeleton.bone_names.iter().collect::<HashSet<_>>().len() == n,
            "ambiguous bone names within {path}"
        );
        let bind = assets::world_matrices(&skeleton, &SkeletonPose::rest_pose(&skeleton));
        let base_bind = assets::world_matrices(base, &SkeletonPose::rest_pose(base));
        let anchors = (0..n)
            .map(|mut i| {
                // Match the base branch as well as the name; a coincident name in
                // a different branch is not a shared attachment.
                loop {
                    if let Some(b) = base.bone_index(&skeleton.bone_names[i]) {
                        let (mut pi, mut bi) = (i, b);
                        loop {
                            let Ok(parent) = usize::try_from(skeleton.parent_indices[pi]) else {
                                return Ok(b);
                            };
                            let Ok(bp) = usize::try_from(base.parent_indices[bi]) else {
                                break;
                            };
                            if base.bone_names[bp] != skeleton.bone_names[parent] {
                                break;
                            }
                            (pi, bi) = (parent, bp);
                        }
                    }
                    if skeleton.parent_indices[i] < 0
                        && let Some(b) = connection
                        && let Some(parent) = base_bind.get(b)
                        && bind[i]
                            .iter()
                            .zip(parent)
                            .all(|(x, y)| (x - y).abs() < 1e-4)
                    {
                        return Ok(b);
                    }
                    i = usize::try_from(skeleton.parent_indices[i]).with_context(|| {
                        format!("no base anchor for {} in {path}", skeleton.bone_names[i])
                    })?;
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let inverse: Vec<_> = bind
            .into_iter()
            .map(|m| Mat4::from_cols_array(&m).inverse())
            .collect();
        ensure!(
            inverse.iter().all(|m| m.is_finite()),
            "singular partial bind in {path}"
        );
        Ok(Self {
            path,
            skeleton,
            anchors,
            inverse,
            delta: vec![Mat4::IDENTITY; n],
            playback: None,
        })
    }
    fn pose(&mut self, pose: &SkeletonPose) -> Result<()> {
        let d = assets::world_matrices(&self.skeleton, pose)
            .iter()
            .zip(&self.inverse)
            .map(|(m, inv)| Mat4::from_cols_array(m) * *inv)
            .collect::<Vec<_>>();
        ensure!(d.iter().all(|m| m.is_finite()), "partial joint overflow");
        self.delta = d;
        Ok(())
    }
}

/// Look up the actual named PAP record and follow its Havok container binding.
/// Only accept transform maps explicitly authored for this partial SKLB.
pub(super) struct PartialClip {
    spline: SplineAnimation,
    tracks: Vec<Option<usize>>,
    additive: bool,
}
impl PartialClip {
    fn load(r: &PartialRig, pap: &[u8], name: &str) -> Result<Self> {
        ensure!(
            (26..=super::MAX_FILE_BYTES).contains(&pap.len())
                && pap.starts_with(b"pap \x01\x00\x02\x00"),
            "invalid partial PAP header"
        );
        let u32at = |o: usize| u32::from_le_bytes(pap[o..o + 4].try_into().unwrap()) as usize;
        let records = u32at(14);
        let havok = u32at(18);
        let end = u32at(22);
        let count = u16::from_le_bytes(pap[8..10].try_into()?);
        ensure!(
            records >= 26
                && records
                    .checked_add(40 * usize::from(count))
                    .is_some_and(|n| n <= havok)
                && havok <= end
                && end <= pap.len(),
            "invalid PAP sections"
        );
        let mut entries = pap[records..records + 40 * usize::from(count)]
            .chunks_exact(40)
            .filter(|row| row[..32].split(|&b| b == 0).next() == Some(name.as_bytes()));
        let record = entries.next().context("partial PAP record not found")?;
        ensure!(entries.next().is_none(), "ambiguous partial PAP name");
        let idx = usize::from(u16::from_le_bytes(record[34..36].try_into()?));
        let graph = Tagfile::parse(&pap[havok..end])?;
        let container = graph.animation_container()?;
        let obj = |field: &str| {
            graph
                .objects(container, field)
                .and_then(|a| a.get(idx))
                .copied()
                .context("missing PAP container reference")
        };
        let b = obj("bindings")?;
        let a = obj("animations")?;
        ensure!(
            graph.type_name(b) == Some("hkaAnimationBinding")
                && graph.object_value(b, "animation") == Some(a)
                && graph.type_name(a) == Some("hkaSplineCompressedAnimation"),
            "invalid partial PAP binding"
        );
        let id = r
            .path
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_start_matches("skl_")
            .trim_end_matches(".sklb");
        let original = graph
            .string(b, "originalSkeletonName")
            .context("partial clip has no skeleton identity")?;
        let (owner, root) = original
            .split_once("_0:mdl:")
            .context("partial clip has no qualified rig identity")?;
        ensure!(
            owner == id
                && r.skeleton
                    .bone_index(root)
                    .is_some_and(|i| r.skeleton.parent_indices[i] < 0),
            "PAP targets {original}, not {}",
            r.path
        );
        let blend = graph.int(b, "blendHint").unwrap_or(0);
        ensure!(
            matches!(blend, 0 | 1),
            "unsupported partial blend hint {blend}"
        );
        let spline = SplineAnimation::from_tagfile(&graph, a)?;
        let map = graph
            .ints(b, "transformTrackToBoneIndices")
            .context("missing partial track map")?;
        ensure!(
            map.len() == spline.number_of_transform_tracks,
            "invalid partial track count"
        );
        let mut used = HashSet::new();
        let tracks = map
            .iter()
            .map(|&v| {
                if v == -1 {
                    return Ok(None);
                }
                let i = usize::try_from(v)?;
                ensure!(
                    i < r.skeleton.bone_count() && used.insert(i),
                    "invalid partial track destination"
                );
                Ok(Some(i))
            })
            .collect::<Result<_>>()?;
        spline.validate()?;
        Ok(Self {
            spline,
            tracks,
            additive: blend == 1,
        })
    }
    fn sample(&self, sk: &ModelSkeleton, t: f32) -> Result<SkeletonPose> {
        let mut pose = SkeletonPose::rest_pose(sk);
        let values = self.spline.sample(t * 1000.)?;
        ensure!(
            values.len() == self.tracks.len(),
            "partial sample size mismatch"
        );
        for (s, &i) in values.iter().zip(&self.tracks) {
            let Some(i) = i else { continue };
            let q = Quat::from_array(s.rotation);
            ensure!(
                q.is_finite() && q.length_squared() > f32::EPSILON,
                "invalid partial quaternion"
            );
            let q = q.normalize();
            let (mut t, mut scale, mut rot) = (
                Vec3::from_slice(&s.translation),
                Vec3::from_slice(&s.scale),
                q,
            );
            if self.additive {
                let rest = sk.rest_pose[i];
                t += q * Vec3::from(rest.translation);
                scale *= Vec3::from(rest.scale);
                rot = q * Quat::from_array(rest.rotation).normalize();
            }
            ensure!(
                t.is_finite() && scale.is_finite() && rot.is_finite(),
                "nonfinite partial sample"
            );
            pose.set_transform(
                i,
                BoneTransform {
                    translation: t.into(),
                    rotation: rot.to_array(),
                    scale: scale.into(),
                },
            )?;
        }
        Ok(pose)
    }
}

impl Actor {
    pub(super) fn skin(&self, worlds: &[[f32; 16]]) -> Result<Vec<[f32; 16]>> {
        let m = self
            .palette
            .iter()
            .map(|&(i, inv, partial)| {
                let skin = Mat4::from_cols_array(&worlds[i]) * inv;
                // Partial rest cancels exactly: baseSkin(anchor) * partialSkin(joint).
                partial
                    .map_or(skin, |(r, b)| skin * self.partials[r].delta[b])
                    .to_cols_array()
            })
            .collect::<Vec<_>>();
        ensure!(
            m.iter().flatten().all(|v| v.is_finite()),
            "actor joint overflow"
        );
        Ok(m)
    }
    fn rig(&mut self, path: &str) -> Result<&mut PartialRig> {
        self.partials
            .iter_mut()
            .find(|r| r.path == path)
            .context("partial SKLB is not attached to this actor")
    }
}

impl EffectPlayer {
    pub fn actor_partial_rigs(&self) -> Vec<String> {
        self.actor
            .iter()
            .flat_map(|a| a.partials.iter().map(|r| r.path.clone()))
            .collect()
    }
    pub fn actor_partial_skeleton(&self, path: &str) -> Option<&ModelSkeleton> {
        self.actor
            .as_ref()?
            .partials
            .iter()
            .find(|r| r.path == path)
            .map(|r| &r.skeleton)
    }
    fn upload_partial_pose(&mut self) -> Result<()> {
        let a = self.actor.as_mut().context("no actor attached")?;
        a.mesh
            .update_joint_matrices(self.renderer.context(), &a.skin(&a.worlds)?);
        Ok(())
    }
    /// Full parent-local T3/Q4/S3 rows in this partial SKLB's own order.
    pub fn set_actor_partial_locals(&mut self, path: &str, trs: &[f32]) -> Result<()> {
        let r = self
            .actor
            .as_mut()
            .context("no actor attached")?
            .rig(path)?;
        let rest = r
            .skeleton
            .rest_pose
            .iter()
            .flat_map(|b| b.translation.into_iter().chain(b.rotation).chain(b.scale));
        if rest.eq(trs.iter().copied()) {
            r.delta.fill(Mat4::IDENTITY);
        } else {
            r.pose(&Self::decode_actor_locals(&r.skeleton, trs)?)?;
        }
        r.playback = None;
        self.upload_partial_pose()
    }
    pub fn clear_actor_partial(&mut self, path: &str) -> Result<()> {
        let r = self
            .actor
            .as_mut()
            .context("no actor attached")?
            .rig(path)?;
        r.playback = None;
        r.delta.fill(Mat4::IDENTITY);
        self.upload_partial_pose()
    }
    pub fn set_actor_partial_clip(
        &mut self,
        path: &str,
        pap: &[u8],
        name: &str,
        looping: bool,
    ) -> Result<f32> {
        let r = self
            .actor
            .as_mut()
            .context("no actor attached")?
            .rig(path)?;
        let clip = PartialClip::load(r, pap, name)?;
        let duration = clip.spline.duration;
        r.pose(&clip.sample(&r.skeleton, 0.)?)?;
        r.playback = Some((clip, self.host_tick, looping));
        self.upload_partial_pose()?;
        Ok(duration)
    }
    pub(crate) fn sample_actor_partials(&mut self) -> Result<()> {
        let Some(a) = self.actor.as_mut() else {
            return Ok(());
        };
        let mut changed = false;
        for r in &mut a.partials {
            let Some((clip, start, looping)) = &r.playback else {
                continue;
            };
            let t = self.host_tick.saturating_sub(*start) as f64 / 60.;
            let d = f64::from(clip.spline.duration);
            let pose = clip.sample(&r.skeleton, if *looping { t % d } else { t.min(d) } as f32)?;
            r.pose(&pose)?;
            changed = true;
        }
        if changed {
            self.upload_partial_pose()?;
        }
        Ok(())
    }
    pub(crate) fn rewind_actor_partials(&mut self) {
        if let Some(a) = &mut self.actor {
            for (_, start, _) in a.partials.iter_mut().filter_map(|r| r.playback.as_mut()) {
                *start = 0;
            }
        }
    }
}
