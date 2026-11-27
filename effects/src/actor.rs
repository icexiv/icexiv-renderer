//! Dressed character and held weapon share one render context, but not a joint palette.
use super::EffectPlayer;
use anyhow::{Context, Result, ensure};
use assets::{
    BoneTransform, DressedCharacterLoadRequest, ModelSkeleton, SkeletonPose, WeaponModelData,
};
use glam::{Mat4, Quat};
use scene::{ModelInstance, PreparedModelOptions};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
mod est;
mod partial;
use partial::PartialRig;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorPiece {
    pub model_main: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_sub: Option<String>,
    pub equip_slot_category: u32,
    #[serde(default)]
    pub item_id: u32,
    #[serde(default)]
    pub stain_ids: [u8; 2],
}

#[derive(Serialize, Deserialize)]
pub struct ActorRequest {
    pub customize: [u8; 26],
    #[serde(default)]
    pub equipment: Vec<ActorPiece>,
}
impl ActorRequest {
    pub fn dressed(&self) -> Result<DressedCharacterLoadRequest> {
        ensure!(
            self.equipment.len() <= 16,
            "too many actor equipment pieces"
        );
        let customize =
            assets::CharacterCustomize::from_bytes(&self.customize).map_err(anyhow::Error::msg)?;
        customize.validate().map_err(anyhow::Error::msg)?;
        let equipment = self
            .equipment
            .iter()
            .map(|p| {
                Ok(assets::DressedEquipmentPiece {
                    model_main: p.model_main.parse().context("invalid packed modelMain")?,
                    model_sub: p
                        .model_sub
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .context("invalid packed modelSub")?,
                    item_id: p.item_id,
                    item_name: String::new(),
                    equip_slot_category: p.equip_slot_category,
                    stain_ids: p.stain_ids,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DressedCharacterLoadRequest::new(customize, "actor").with_equipment(equipment))
    }
}

pub const MAX_FILES: usize = 1024;
pub const MAX_FILE_BYTES: usize = 64 << 20;
pub const MAX_BUNDLE_BYTES: usize = 128 << 20;

pub struct ActorAssets {
    pub model: WeaponModelData,
    pub skeleton: ModelSkeleton,
    pub options: PreparedModelOptions,
    partials: Vec<PartialRig>,
    bind_points: Vec<assets::avfx::VfxBindPoint>,
}

#[inline(never)]
fn appearance_from_resource<R: physis::resource::Resource>(
    source: &mut R,
    customize: &assets::CharacterCustomize,
) -> Option<assets::CharacterAppearanceColors> {
    let bytes = source.read("chara/xls/charamake/human.cmp").ok()?;
    let cmp =
        <physis::cmp::CMP as physis::ReadableFile>::from_existing(physis::Platform::Win32, &bytes)
            .ok()?;
    let palette = assets::character_palette_from_cmp(&cmp);
    let mut appearance = assets::appearance_colors_from_palette(customize, &palette);
    // Use the shader parameter palettes (`skin`, `hair.main`) rather than the
    // darker UI color-picker swatches (`skin_interface`, `hair_interface`).
    if let Some(race) = cmp.races.get(customize.palette_group_index()) {
        let skin = race.skin[usize::from(customize.skintone).min(race.skin.len() - 1)];
        appearance.skin = [
            f32::from(skin.red) / 255.0,
            f32::from(skin.green) / 255.0,
            f32::from(skin.blue) / 255.0,
            f32::from(skin.alpha) / 255.0,
        ];
        let hair = race.hair[usize::from(customize.hair_tone).min(race.hair.len() - 1)].main;
        appearance.main = [
            f32::from(hair.red) / 255.0,
            f32::from(hair.green) / 255.0,
            f32::from(hair.blue) / 255.0,
            f32::from(hair.alpha) / 255.0,
        ];
    }
    Some(appearance)
}

impl ActorAssets {
    pub fn load<R: physis::resource::Resource>(
        source: &mut R,
        request: &DressedCharacterLoadRequest,
    ) -> Result<Self> {
        let mut slots = HashSet::new();
        for piece in &request.equipment {
            ensure!(
                assets::equipment_slot_info(piece.equip_slot_category).is_some()
                    && slots.insert(piece.equip_slot_category),
                "unsupported or repeated dressed-actor slot {}",
                piece.equip_slot_category
            );
        }
        let resolved;
        let request = if request.appearance.is_none()
            && let Some(appearance) = appearance_from_resource(source, &request.customize)
        {
            resolved = request.clone().with_appearance(appearance);
            &resolved
        } else {
            request
        };
        let (mut data, skeleton) =
            assets::load_dressed_character_with_skeleton_from_resource(source, request)?;
        for piece in &request.equipment {
            ensure!(
                data.equipment_material_ranges
                    .iter()
                    .any(|r| r.item_id == piece.item_id
                        && r.equip_slot_category == piece.equip_slot_category),
                "could not load requested actor equipment {} (slot {})",
                piece.model_main,
                piece.equip_slot_category
            );
        }
        let skeleton = skeleton.context("actor base SKLB not loaded")?;
        let partials = bind_partial_rigs(
            source,
            &mut data.model,
            &skeleton,
            request.customize.race_code(),
        )?;
        let visible = assets::character_enabled_attribute_names(&request.customize, &data.model);
        let mut seen_points = HashSet::new();
        let mut bind_points = Vec::new();
        for path in &data.model.loaded_paths {
            if path.ends_with(".mdl")
                && let Ok(bytes) = source.read(path)
                && let Ok(pts) = super::pose::mdl_points(&bytes)
            {
                for pt in pts {
                    if seen_points.insert(pt.id) {
                        bind_points.push(pt);
                    }
                }
            }
        }
        Ok(Self {
            model: data.model,
            skeleton,
            partials,
            bind_points,
            options: PreparedModelOptions::default()
                .with_component_preview_layout(false)
                .with_enabled_attribute_names(visible),
        })
    }
}

/// Qualify partial bone names by source SKLB. Base-only mesh names remain
/// unchanged, even when a face or hair contains an identically named joint.
fn bind_partial_rigs<R: physis::resource::Resource>(
    source: &mut R,
    model: &mut WeaponModelData,
    base: &ModelSkeleton,
    actor_race: u16,
) -> Result<Vec<PartialRig>> {
    let mut cache: HashMap<String, Option<ModelSkeleton>> = HashMap::new();
    let mut templates = est::Templates::default();
    let mut connections = HashMap::new();
    for mesh in &mut model.meshes {
        let Some(table) = &mut mesh.bone_table else {
            continue;
        };
        let mut used = vec![false; table.bone_names.len()];
        for v in &mesh.vertices {
            if let (Some(w), Some(b)) = (v.blend_weights, v.blend_indices) {
                for i in 0..usize::from(w.count).min(8) {
                    if w.values[i] > 0. {
                        *used
                            .get_mut(usize::from(b.values[i]))
                            .context("actor vertex outside bone palette")? = true;
                    }
                }
            }
        }
        let path = mesh.path.split('#').next().unwrap_or("");
        let mut candidates = Vec::new();
        if let Some(rest) = path.strip_prefix("chara/equipment/e")
            && let Some((set, file)) = rest.split_once("/model/")
            && let Ok(set) = set.parse()
            && let Some(kind @ ("met" | "top")) =
                file.strip_suffix(".mdl").and_then(|s| s.rsplit('_').next())
            && let Some(Some(p)) = templates.lookup(source, kind, actor_race, set)?
        {
            candidates.push(p);
        }
        if let Some((root, rest)) = path.split_once("/obj/")
            && let Some(race) = root.strip_prefix("chara/human/")
        {
            let mut parts = rest.split('/');
            if let (Some(kind @ ("face" | "hair" | "zear" | "tail")), Some(id)) =
                (parts.next(), parts.next())
            {
                let id_path = |id: &str| format!("{root}/skeleton/{kind}/{id}/skl_{race}{id}.sklb");
                if let Some(template) = id
                    .get(1..)
                    .and_then(|s| s.parse().ok())
                    .map(|set| templates.lookup(source, kind, actor_race, set))
                    .transpose()?
                    .flatten()
                {
                    candidates.extend(template);
                } else {
                    candidates.push(id_path(id));
                    // Hair model numbers and extra-skeleton numbers may differ.
                    for extra in table.bone_names.iter().flatten().filter_map(|name| {
                        name.strip_prefix("j_ex_").and_then(|s| s.split('_').next())
                    }) {
                        if extra.len() == 5
                            && extra.as_bytes()[0].is_ascii_alphabetic()
                            && extra.bytes().next() == id.bytes().next()
                            && extra[1..].bytes().all(|b| b.is_ascii_digit())
                            && !candidates.contains(&id_path(extra))
                        {
                            candidates.push(id_path(extra));
                        }
                    }
                }
            }
        }
        for p in &candidates {
            if !cache.contains_key(p) {
                let partial = match source.read(p) {
                    Ok(bytes) => {
                        if bytes.get(..8) == Some(b"blks0031")
                            && let Some(index) = bytes.get(16..18)
                        {
                            connections.insert(
                                p.clone(),
                                usize::from(u16::from_le_bytes(index.try_into()?)),
                            );
                        }
                        Some(
                            assets::load_skeleton_from_sklb_bytes(&bytes)
                                .with_context(|| p.clone())?,
                        )
                    }
                    Err(physis::Error::FileNotFound { .. }) => None,
                    Err(e) => return Err(e.into()),
                };
                cache.insert(p.clone(), partial);
            }
        }
        for (slot, name) in table.bone_names.iter_mut().enumerate() {
            if !used[slot] {
                *name = None;
                continue;
            }
            let name = name.as_mut().context("weighted actor joint lacks name")?;
            let mut matches = candidates
                .iter()
                .filter_map(|p| cache[p].as_ref()?.bone_index(name).map(|i| (p, i)));
            if let Some((p, i)) = matches.next() {
                ensure!(
                    matches.next().is_none(),
                    "ambiguous partial rig for {name} in {path}"
                );
                *name = partial::key(p, i);
                continue;
            }
            ensure!(
                base.bone_index(name).is_some(),
                "unsupported weighted actor joint {name} in {path}: matching partial SKLB required"
            );
        }
    }
    let joints: HashSet<_> = model
        .meshes
        .iter()
        .filter_map(|m| m.bone_table.as_ref())
        .flat_map(|t| t.bone_names.iter().flatten())
        .collect();
    ensure!(
        joints.len() <= 256,
        "actor exceeds the scene's 256-joint skin palette"
    );
    let mut rigs = vec![];
    for (path, sk) in cache {
        if let Some(sk) = sk
            && (0..sk.bone_count()).any(|i| joints.contains(&partial::key(&path, i)))
        {
            let connection = connections.get(&path).copied();
            rigs.push(PartialRig::new(path, sk, base, connection)?);
        }
    }
    rigs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rigs)
}

pub(super) struct Actor {
    pub(super) mesh: ModelInstance,
    skeleton: ModelSkeleton,
    palette: Vec<(usize, Mat4, Option<(usize, usize)>)>,
    partials: Vec<PartialRig>,
    bind_points: Vec<assets::avfx::VfxBindPoint>,
    last_pose: SkeletonPose,
    worlds: Vec<[f32; 16]>,
    model_worlds: Vec<[f32; 16]>,
    placement: Mat4,
    bind_sphere: ([f32; 3], f32),
    hand: usize,
    left_hand: Option<usize>,
    left_world: Option<[f32; 16]>,
    grip: [f32; 16],
}

impl Actor {
    pub(super) fn action_elements(&self) -> Result<Vec<assets::VfxBinderTargetSnapshot>> {
        if self.bind_points.is_empty() {
            return Ok(Vec::new());
        }
        let mut elements = assets::VfxRuntime::bind_point_pose_targets(
            &self.bind_points,
            &self.skeleton,
            &self.last_pose,
        )
        .map_err(anyhow::Error::msg)?;
        if self.placement != Mat4::IDENTITY {
            let linear = glam::Mat3::from_mat4(self.placement);
            for e in &mut elements {
                let p = self
                    .placement
                    .transform_point3(glam::Vec3::from(e.matrix.position));
                e.matrix.position = p.to_array();
                for col in &mut e.matrix.basis {
                    *col = (linear * glam::Vec3::from(*col)).to_array();
                }
            }
        }
        Ok(elements)
    }
}

impl EffectPlayer {
    /// Native held-weapon entry point. Ordinary weapons need their own SKLB
    /// even when their IMC variant carries no resident effect.
    pub fn new_held_weapon<R: physis::resource::Resource>(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        source: &mut R,
        main: u64,
    ) -> Result<Self> {
        let player = Self::new(
            device,
            queue,
            format,
            super::EffectAssets::rigged(source, main, true)?,
        );
        ensure!(
            !player.renderer.joint_names().is_empty(),
            "weapon MDL has no joint palette"
        );
        Ok(player)
    }
    /// Attach the assembled actor at its scaled rest pose. `grip` maps weapon
    /// space into the local frame of `hand`. Weapon rig must already be prepared.
    pub fn set_actor(&mut self, data: ActorAssets, hand: &str, grip: [f32; 16]) -> Result<()> {
        let pose = data.skeleton.scaled_rest_pose();
        self.install_actor(data, hand, grip, &pose)
    }
    fn install_actor(
        &mut self,
        data: ActorAssets,
        hand: &str,
        grip: [f32; 16],
        pose: &SkeletonPose,
    ) -> Result<()> {
        ensure!(
            !self.renderer.joint_names().is_empty(),
            "weapon MDL has no prepared skin palette"
        );
        let hand = data
            .skeleton
            .bone_index(hand)
            .context("actor grip bone missing")?;
        let left_hand = data.skeleton.bone_index("n_buki_l");
        ensure!(
            self.offhand.is_none() || left_hand.is_some(),
            "equipped offhand requires n_buki_l on replacement actor"
        );
        let mesh = self.renderer.context().create_model_with_skeleton(
            &data.model,
            data.options.with_component_preview_layout(false),
            Some(&data.skeleton),
        );
        ensure!(mesh.joint_count() > 0, "actor has no skinned meshes");
        let bind = assets::world_matrices(&data.skeleton, &SkeletonPose::rest_pose(&data.skeleton));
        let mut partial_slots = HashMap::new();
        for (r, rig) in data.partials.iter().enumerate() {
            for (i, &anchor) in rig.anchors.iter().enumerate() {
                partial_slots.insert(partial::key(&rig.path, i), (anchor, (r, i)));
            }
        }
        let palette = mesh
            .joint_names()
            .iter()
            .map(|name| {
                let (i, partial) = if let Some(&(i, slot)) = partial_slots.get(name) {
                    (i, Some(slot))
                } else {
                    (
                        data.skeleton
                            .bone_index(name)
                            .with_context(|| format!("actor palette bone missing: {name}"))?,
                        None,
                    )
                };
                let inverse = Mat4::from_cols_array(&bind[i]).inverse();
                ensure!(inverse.is_finite(), "singular actor bind: {name}");
                Ok((i, inverse, partial))
            })
            .collect::<Result<_>>()?;
        let actor = Actor {
            bind_sphere: mesh.bounds_sphere(),
            mesh,
            last_pose: pose.clone(),
            skeleton: data.skeleton,
            palette,
            partials: data.partials,
            bind_points: data.bind_points,
            model_worlds: bind.clone(),
            worlds: bind,
            placement: Mat4::IDENTITY,
            hand,
            left_hand,
            left_world: None,
            grip,
        };
        let previous = self.actor.replace(actor);
        if let Err(e) = self.apply_actor_pose(pose) {
            self.actor = previous;
            return Err(e);
        }
        Ok(())
    }
    pub fn clear_actor(&mut self) -> Result<()> {
        if self.actor.is_some() {
            self.set_local_pose(&self.rest_local_pose())?;
            self.clear_offhand();
            self.actor = None;
        }
        Ok(())
    }
    pub fn actor_skeleton(&self) -> Option<&ModelSkeleton> {
        self.actor.as_ref().map(|a| &a.skeleton)
    }
    pub(super) fn left_hand_world(&self) -> Option<[f32; 16]> {
        self.actor.as_ref().and_then(|a| a.left_world)
    }
    /// Host samples its PAP in frame_with_host. Rows follow actor_skeleton bone
    /// order, not mesh-palette order. Per-character body scaling is applied once.
    pub fn set_actor_locals(&mut self, trs: &[f32]) -> Result<()> {
        let sk = self.actor_skeleton().context("no actor attached")?;
        self.apply_actor_pose(&Self::decode_actor_locals(sk, trs)?)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn install_actor_locals(
        &mut self,
        data: ActorAssets,
        hand: &str,
        grip: [f32; 16],
        trs: &[f32],
    ) -> Result<()> {
        let pose = Self::decode_actor_locals(&data.skeleton, trs)?;
        self.install_actor(data, hand, grip, &pose)
    }

    fn decode_actor_locals(sk: &ModelSkeleton, trs: &[f32]) -> Result<SkeletonPose> {
        ensure!(trs.len() == 10 * sk.bone_count(), "wrong actor pose length");
        let mut pose = SkeletonPose::new(sk.bone_count());
        for (i, row) in trs.chunks_exact(10).enumerate() {
            let q = Quat::from_slice(&row[3..]);
            ensure!(
                row.iter().all(|v| v.is_finite())
                    && q.length_squared().is_finite()
                    && q.length_squared() > f32::EPSILON,
                "invalid actor transform"
            );
            pose.set_transform(
                i,
                BoneTransform {
                    translation: row[..3].try_into()?,
                    rotation: q.normalize().to_array(),
                    scale: row[7..].try_into()?,
                },
            )?;
        }
        if let Some(scaling) = sk.body_scaling {
            pose.apply_body_scaling(sk, &scaling);
        }
        Ok(pose)
    }
    /// Apply a fully sampled pose (including any character body scaling).
    pub fn apply_actor_pose(&mut self, pose: &SkeletonPose) -> Result<()> {
        let a = self.actor.as_mut().context("no actor attached")?;
        ensure!(
            pose.bone_count() == a.skeleton.bone_count(),
            "wrong actor pose length"
        );
        let model_worlds = assets::world_matrices(&a.skeleton, pose);
        a.last_pose = pose.clone();
        let placement = a.placement;
        self.install_actor_worlds(model_worlds, placement)
    }
    /// Absolute scene transform, maintained across clips. The host may supply
    /// locomotion or recorded object transforms before any simulation tick.
    pub fn set_actor_placement(&mut self, position: [f32; 3], rotation: [f32; 4]) -> Result<()> {
        let q = Quat::from_array(rotation);
        ensure!(
            position.into_iter().all(f32::is_finite)
                && q.length_squared().is_finite()
                && q.length_squared() > f32::EPSILON,
            "invalid actor placement"
        );
        let m = Mat4::from_rotation_translation(q.normalize(), position.into());
        let a = self.actor.as_ref().context("no actor attached")?;
        if a.placement == m {
            return Ok(());
        }
        self.install_actor_worlds(a.model_worlds.clone(), m)
    }
    fn install_actor_worlds(
        &mut self,
        model_worlds: Vec<[f32; 16]>,
        placement: Mat4,
    ) -> Result<()> {
        let a = self.actor.as_ref().context("no actor attached")?;
        let worlds: Vec<_> = model_worlds
            .iter()
            .map(|w| (placement * Mat4::from_cols_array(w)).to_cols_array())
            .collect();
        let matrices = a.skin(&worlds)?;
        let hand = worlds[a.hand];
        let left_hand = a.left_hand.map(|i| worlds[i]);
        let grip = a.grip;
        self.set_weapon_grip(&hand, &grip)?;
        self.update_offhand_grip(left_hand)?;
        let a = self.actor.as_mut().unwrap();
        a.model_worlds = model_worlds;
        a.placement = placement;
        a.mesh.set_bounds_sphere(
            placement
                .transform_point3(a.bind_sphere.0.into())
                .to_array(),
            a.bind_sphere.1,
        );
        a.left_world = left_hand;
        a.worlds = worlds;
        a.mesh
            .update_joint_matrices(self.renderer.context(), &matrices);
        Ok(())
    }
}
