//! Optional real-data GPU acceptance. XIV_GAME_DIR is the directory containing sqpack.
#[path = "support/offhand.rs"]
mod offhand;
use anyhow::{Context, Result, ensure};
use assets::{CharacterCustomize, DressedCharacterLoadRequest, DressedEquipmentPiece};
use icexiv_effects::{EffectAssets, EffectPlayer, actor::ActorAssets};
use physis::resource::{Resource, SqPackResource};
use std::{collections::BTreeMap, path::PathBuf};

// Optionally record the same actor file closure for the browser acceptance.
#[derive(Clone)]
struct Source {
    game: SqPackResource,
    out: Option<PathBuf>,
    files: BTreeMap<String, usize>,
}
impl Resource for Source {
    fn read(&mut self, path: &str) -> physis::Result<Vec<u8>> {
        let bytes = self.game.read(path)?;
        if let Some(out) = &self.out {
            if !self.files.contains_key(path) {
                let i = self.files.len();
                std::fs::write(out.join(i.to_string()), &bytes).unwrap();
                self.files.insert(path.into(), i);
            }
        }
        Ok(bytes)
    }
    fn exists(&mut self, path: &str) -> bool {
        self.game.exists(path)
    }
}

#[test]
#[ignore = "needs XIV_GAME_DIR and a GPU adapter"]
fn dressed_idle_holds_a_saw_in_the_effect_pass() -> Result<()> {
    // Debug-format texture decoders exceed libtest's small default stack.
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run_fixture)?
        .join()
        .expect("actor acceptance thread panicked")
}
fn run_fixture() -> Result<()> {
    let dir = std::env::var("XIV_GAME_DIR")?;
    let out = std::env::var_os("ACTOR_FIXTURE_OUT").map(PathBuf::from);
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir)?;
    }
    let mut game = SqPackResource::from_existing(&dir);
    let mut source = Source {
        game: game.clone(),
        out: out.clone(),
        files: Default::default(),
    };
    let customize = CharacterCustomize {
        race: 1,
        gender: 0,
        age: 1,
        height: 50,
        tribe: 1,
        head: 1,
        hair: 1,
        ..Default::default()
    };
    // c0101, Spring Dress torso (e0141), Saw of the Stars main component.
    let req = DressedCharacterLoadRequest::new(customize, "actor").with_equipment(vec![
        DressedEquipmentPiece {
            item_id: 7540,
            item_name: "Spring Dress".into(),
            model_main: 65677,
            model_sub: 0,
            equip_slot_category: 4,
            stain_ids: [0; 2],
        },
    ]);
    let actor = ActorAssets::load(&mut source, &req)?;
    ensure!(
        actor.model.meshes.iter().any(|m| m.path.contains("e0141")),
        "equipment absent"
    );
    ensure!(
        actor.model.meshes.iter().any(|m| m.path.contains("/face/")),
        "body absent"
    );
    if let Some(dir) = &out {
        std::fs::write(dir.join("files.json"), serde_json::to_vec(&source.files)?)?;
        std::fs::write(dir.join("customize.bin"), customize.to_bytes())?;
    }
    let pap = game.read("chara/human/c0101/animation/a0001/bt_common/resident/idle.pap")?;
    let set = assets::load_animation_set_from_pap_bytes(&pap, &actor.skeleton)?;
    let index = set
        .animations
        .iter()
        .position(|a| a.name == "cbnm_id0")
        .context("idle not decoded")?;
    let mut weapon = Source {
        game: game.clone(),
        out: out.as_ref().map(|d| d.join("weapon")),
        files: Default::default(),
    };
    if let Some(dir) = &weapon.out {
        std::fs::create_dir_all(dir)?;
    }
    let saw = EffectAssets::load(&mut weapon, 17180398476)?;
    if let Some(dir) = &out {
        std::fs::write(
            dir.join("weapon.json"),
            serde_json::to_vec(&serde_json::json!({"main":"17180398476","files":weapon.files}))?,
        )?;
        std::fs::write(dir.join("idle.pap"), &pap)?;
    }
    let inst = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(inst.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: scene::ModelRenderContext::required_limits(adapter.limits()),
        ..Default::default()
    }))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut player = EffectPlayer::new(device.clone(), queue.clone(), format, saw);
    const SIDE: u32 = 256;
    let size = wgpu::Extent3d {
        width: SIDE,
        height: SIDE,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        format,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let pixels = || -> Result<Vec<u8>> {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIDE * SIDE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * SIDE),
                    rows_per_image: Some(SIDE),
                },
            },
            size,
        );
        let idx = queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(idx),
            timeout: None,
        })?;
        rx.recv()??;
        Ok(buf.slice(..).get_mapped_range().to_vec())
    };
    let draw = |p: &mut EffectPlayer| p.frame(&view, [SIDE; 2], 0., 0., 3.2, 0., false);
    draw(&mut player)?;
    let standalone = pixels()?;
    let grip = glam::Mat4::IDENTITY.to_cols_array();
    player.set_actor(actor, "n_buki_r", grip)?;
    let pose_at = |p: &EffectPlayer, ms| {
        assets::sample_animation_pose(&set, index, ms, p.actor_skeleton().unwrap())
    };
    let pose = pose_at(&player, 125.);
    player.apply_actor_pose(&pose)?;
    draw(&mut player)?;
    let both = pixels()?;
    // Move only the held weapon offscreen: same instance list, same fitted camera.
    player.set_weapon_grip(
        &glam::Mat4::from_translation(glam::Vec3::splat(100.)).to_cols_array(),
        &grip,
    )?;
    draw(&mut player)?;
    let body = pixels()?;
    let changed = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a[..3] != b[..3])
            .count()
    };
    ensure!(changed(&body, &both) > 8, "weapon not visible");
    let pose = pose_at(&player, 875.);
    player.apply_actor_pose(&pose)?;
    player.set_weapon_grip(
        &glam::Mat4::from_translation(glam::Vec3::splat(100.)).to_cols_array(),
        &grip,
    )?;
    draw(&mut player)?;
    ensure!(changed(&body, &pixels()?) > 8, "actor mesh did not animate");
    {
        let face = "chara/human/c0101/skeleton/face/f0002/skl_c0101f0002.sklb";
        let hair = "chara/human/c0101/skeleton/hair/h0003/skl_c0101h0003.sklb";
        let base_names = player.actor_skeleton().unwrap().bone_names.clone();
        let sk = player.actor_partial_skeleton(face).context("no face rig")?;
        let packed = |s: &assets::ModelSkeleton| {
            s.rest_pose
                .iter()
                .flat_map(|b| b.translation.into_iter().chain(b.rotation).chain(b.scale))
                .collect::<Vec<_>>()
        };
        let face_rest = packed(sk);
        let mut jaw = face_rest.clone();
        jaw[10 * sk.bone_index("j_f_ago").context("jaw joint")? + 1] += 0.06;
        let hair_sk = player.actor_partial_skeleton(hair).context("no hair rig")?;
        let mut moved_hair = packed(hair_sk);
        moved_hair[10 * hair_sk.bone_index("j_kao").context("hair attachment")?] += 0.035;
        let original = pixels()?;
        player.set_actor_partial_locals(hair, &moved_hair)?;
        draw(&mut player)?;
        let hair_only = pixels()?;
        ensure!(
            changed(&original, &hair_only) > 0,
            "hair palette did not move"
        );
        player.set_actor_partial_locals(face, &jaw)?;
        draw(&mut player)?;
        let jaw_pixels = changed(&hair_only, &pixels()?);
        ensure!(jaw_pixels > 0, "face jaw did not affect rendered pixels");
        player.set_actor_partial_locals(face, &face_rest)?;
        draw(&mut player)?;
        ensure!(
            pixels()? == hair_only,
            "clearing face changed the independently posed hair"
        );
        player.clear_actor_partial(hair)?;
        draw(&mut player)?;
        ensure!(
            pixels()? == original,
            "partial clear failed to restore original pixels"
        );
        let pap = game.read("chara/human/c0101/animation/f0002/resident/face.pap")?;
        if let Some(dir) = &out {
            std::fs::write(dir.join("face.pap"), &pap)?;
        }
        ensure!(
            player
                .set_actor_partial_clip(hair, &pap, "cfxb_blink1", false)
                .is_err(),
            "misbound facial PAP to hair"
        );
        let length = player.set_actor_partial_clip(face, &pap, "cfxb_blink1", false)?;
        ensure!(
            (length - 0.26666668).abs() < 1e-4,
            "unexpected blink duration"
        );
        player.frame(&view, [SIDE; 2], 0., 0., 3.2, 0.1, false)?;
        let blink_pixels = changed(&original, &pixels()?);
        ensure!(blink_pixels > 0, "blink did not deform the face");
        player.clear_actor_partial(face)?;
        draw(&mut player)?;
        ensure!(pixels()? == original, "blink rest restore mismatch");
        ensure!(
            base_names == player.actor_skeleton().unwrap().bone_names,
            "changed base C010 order"
        );
        eprintln!(
            "partial validation: jaw {jaw_pixels}, blink {blink_pixels} changed pixels; rest restored"
        );
    }
    for _ in 0..12 {
        player.frame_with_host(&view, [SIDE; 2], 0., 0., 3.2, 1. / 60., true, |p, t| {
            let pose = pose_at(p, (1000. * t) as f32);
            p.apply_actor_pose(&pose)
        })?;
    }
    let lit = pixels()?;
    draw(&mut player)?;
    ensure!(changed(&lit, &pixels()?) > 8, "resident AVFX not visible");
    if let Some(dir) = out {
        let mut encoder =
            png::Encoder::new(std::fs::File::create(dir.join("actor.png"))?, SIDE, SIDE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&lit)?;
    }
    offhand::check(&mut player, &mut game, &view, &pixels)?;
    player.clear_actor()?;
    player.reset();
    draw(&mut player)?;
    ensure!(standalone == pixels()?, "standalone scene not restored");
    drop(player);
    // w0101/b0001: a plain weapon does not require resident AVFX.
    let mut sword = EffectPlayer::new_held_weapon(
        device.clone(),
        queue.clone(),
        format,
        &mut game,
        101 | (1 << 16) | (1 << 32),
    )?;
    draw(&mut sword)?;
    let rest = pixels()?;
    sword.set_weapon_grip(
        &glam::Mat4::from_translation(glam::Vec3::X * 0.15).to_cols_array(),
        &grip,
    )?;
    draw(&mut sword)?;
    ensure!(
        changed(&rest, &pixels()?) > 8,
        "plain weapon grip did not move the mesh"
    );
    let mut equipped = req;
    let mut top = equipped.equipment[0].clone();
    top.model_main = 65636;
    let mut head = top.clone();
    head.equip_slot_category = 3;
    head.model_main = 65543;
    equipped.equipment = vec![top, head];
    let assets = ActorAssets::load(&mut game, &equipped)?;
    sword.set_actor(assets, "n_buki_r", grip)?;
    let equipment_draw = |p: &mut EffectPlayer| p.frame(&view, [SIDE; 2], 1.2, 0., 5., 0., false);
    equipment_draw(&mut sword)?;
    let equipped_rest = pixels()?;
    for (kind, rig, joint) in [
        ("top", "t0100", "j_ex_top_a"),
        ("met", "m0007", "j_ex_met_a"),
    ] {
        let path = format!("chara/human/c0101/skeleton/{kind}/{rig}/skl_c0101{rig}.sklb");
        let sk = sword
            .actor_partial_skeleton(&path)
            .context("missing equipment rig")?;
        let mut local: Vec<_> = sk
            .rest_pose
            .iter()
            .flat_map(|t| t.translation.into_iter().chain(t.rotation).chain(t.scale))
            .collect();
        local[10 * sk.bone_index(joint).context("equipment joint")?] += 0.25;
        sword.set_actor_partial_locals(&path, &local)?;
        equipment_draw(&mut sword)?;
        ensure!(
            changed(&equipped_rest, &pixels()?) > 0,
            "{joint} not skinned"
        );
        sword.clear_actor_partial(&path)?;
        equipment_draw(&mut sword)?;
        ensure!(equipped_rest == pixels()?, "{joint} did not restore");
    }
    Ok(())
}
