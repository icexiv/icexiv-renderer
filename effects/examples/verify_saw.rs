//! Compare moving w5004/b0008 mesh packets to the same deterministic rest sequence.
//! Usage: verify_saw <51778-main-manifest> <CAS-directory>
use anyhow::{Context, Result, ensure};
use assets::{BoneTransform, SkeletonPose};
use glam::{Mat4, Quat, Vec3};
use icexiv_effects::EffectAssets;
use scene::WeaponVfxPlayback;
use std::{collections::HashMap, fs, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [manifest, root] = args.as_slice() else {
        anyhow::bail!("expected saw manifest and CAS directory")
    };
    let desc: serde_json::Value = serde_json::from_slice(&fs::read(manifest)?)?;
    let mut files = HashMap::new();
    for (path, hash) in desc["files"].as_object().context("missing files")? {
        let hash = hash.as_str().context("invalid digest")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid digest"
        );
        files.insert(
            path.clone(),
            fs::read(root.join(&hash[..2]).join(&hash[2..]))?,
        );
    }
    let assets = EffectAssets::load(
        &mut assets::resource::MemoryResource::new(files),
        17180398476,
    )?;
    let mounts = assets.mounts();
    let [mount] = mounts.attachments.as_slice() else {
        anyhow::bail!("expected one mount")
    };
    let skel = mount.data.skeleton.as_ref().context("no skeleton")?;
    let bone = skel.bone_index("n_hara").context("missing weapon bone")?;
    let mut pose = SkeletonPose::rest_pose(skel);
    let make =
        || WeaponVfxPlayback::from_prepared_aura_inputs(mounts, &[], None, None, |_| Some([0.; 3]));
    let (mut rest, mut moving) = (make(), make());
    let mut max_error = 0f32;
    let mut axis_error = 0f32;
    let mut samples = 0;
    for tick in 0..=240 {
        let t = tick as f32 / 60.;
        let (s, q, p) = (
            Vec3::new(1. + 0.1 * t, 1. + 0.15 * t, 1. + 0.12 * t),
            Quat::from_rotation_z(t * 0.25),
            Vec3::new(0.03 * t, 0.02 * t, 0.01 * t),
        );
        let m = Mat4::from_scale_rotation_translation(s, q, p);
        pose.set_transform(
            bone,
            BoneTransform {
                translation: p.to_array(),
                rotation: q.to_array(),
                scale: s.to_array(),
            },
        )?;
        moving
            .set_attachment_pose(&mount.model_path, &pose)
            .map_err(anyhow::Error::msg)?;
        rest.sample(t, 4096, |_| Some([0.; 3]));
        moving.sample(t, 4096, |_| Some([0.; 3]));
        ensure!(
            rest.fallback_reasons().is_empty() && moving.fallback_reasons().is_empty(),
            "binder fallback"
        );
        if tick < 48 {
            continue;
        }
        let a: Vec<_> = rest
            .meshes()
            .iter()
            .filter(|x| x.model_index == 2)
            .collect();
        let b: Vec<_> = moving
            .meshes()
            .iter()
            .filter(|x| x.model_index == 2)
            .collect();
        ensure!(
            a.len() == 6 && b.len() == 6,
            "missing ring packets at {tick}"
        );
        for (a, b) in a.iter().zip(b) {
            max_error = max_error.max(
                m.transform_point3(a.position.into())
                    .distance(b.position.into()),
            );
            ensure!(
                a.orientation == b.orientation && a.scale == b.scale,
                "unexpected local ring TRS"
            );
            for (av, bv) in a.parent_basis.into_iter().zip(b.parent_basis) {
                axis_error = axis_error.max(m.transform_vector3(av.into()).distance(bv.into()));
            }
            samples += 1;
        }
    }
    println!("{samples} rings; largest centre error {max_error}, basis error {axis_error}");
    ensure!(
        max_error < 1e-4 && axis_error < 1e-4,
        "moving rings failed affine comparison"
    );
    Ok(())
}
