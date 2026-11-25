use anyhow::{Result, ensure};
use icexiv_effects::{
    EffectAssets, EffectPlayer,
    offhand::{OffhandManifest, OffhandOptions},
    timeline::{ActionPlayer, Event, Preview},
};
use physis::resource::{Resource, SqPackResource};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn check(
    player: &mut EffectPlayer,
    game: &mut SqPackResource,
    view: &wgpu::TextureView,
    pixels: impl Fn() -> Result<Vec<u8>>,
) -> Result<()> {
    let frame =
        |p: &mut EffectPlayer, dt, enabled| p.frame(view, [256; 2], 0., 0., 3.2, dt, enabled);
    let diff = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(x, y)| x != y)
            .count()
    };
    frame(player, 0., false)?;
    let alone = pixels()?;
    // Exercise the --owner manifest reader with the real w5081 secondary.
    #[derive(Clone)]
    struct Capture(SqPackResource, BTreeMap<String, Vec<u8>>);
    impl Resource for Capture {
        fn read(&mut self, path: &str) -> physis::Result<Vec<u8>> {
            let b = self.0.read(path)?;
            self.1.entry(path.into()).or_insert_with(|| b.clone());
            Ok(b)
        }
        fn exists(&mut self, path: &str) -> bool {
            self.0.exists(path)
        }
    }
    let mut capture = Capture(game.clone(), BTreeMap::new());
    let owner = EffectAssets::load_owner(&mut capture, 4295037913)?;
    let manifest = OffhandManifest {
        main: "4295037913".into(),
        owner: owner.owner_path().unwrap().into(),
        mounts: vec![],
        files: capture
            .1
            .iter()
            .map(|(p, b)| (p.clone(), format!("{:x}", Sha256::digest(b))))
            .collect(),
    };
    player.set_offhand_bundle(
        &manifest,
        |p, _| Ok(capture.1[p].clone()),
        OffhandOptions::default(),
    )?;
    frame(player, 0., false)?;
    let dual = pixels()?;
    player.set_offhand_visible(false)?;
    frame(player, 0., false)?;
    let hidden = pixels()?;
    let visible = diff(&hidden, &dual);
    ensure!(visible > 8, "secondary draw absent: {visible}");
    player.set_offhand_visible(true)?;
    let pose = player.offhand_elements().unwrap().to_vec();
    ensure!(
        player
            .set_offhand_bundle(
                &manifest,
                |_, _| anyhow::bail!("missing bundle"),
                Default::default()
            )
            .is_err()
    );
    ensure!(
        player.offhand_elements() == Some(&pose[..]),
        "failed replacement mutated live weapon"
    );
    frame(player, 0., false)?;
    ensure!(dual == pixels()?, "failed replacement changed frame");
    player.clear_offhand();
    frame(player, 0., false)?;
    ensure!(alone == pixels()?, "removal did not restore actor and main");

    // Both hands may refer to exactly the same logical MDL. Exercise visible
    // second resident and an independent real spell AVFX on top of both.
    player.set_offhand(game, 17180398476, true, Default::default())?;
    // A left-hand-only pose change must move this hand's ElementIds, not reroute
    // them to the other w5004 instance. Compare against the actual bone world delta.
    let rig = player.actor_skeleton().unwrap().clone();
    let rest = rig.scaled_rest_pose();
    let left = rig.bone_index("n_buki_l").unwrap();
    let mut shifted = rest.clone();
    let mut local = shifted.transform(left).unwrap();
    local.translation[0] += 0.125;
    shifted.set_transform(left, local)?;
    player.apply_actor_pose(&rest)?;
    let before = player.offhand_elements().unwrap().to_vec();
    let placement = glam::Mat4::from_rotation_translation(
        glam::Quat::from_rotation_y(0.8),
        glam::vec3(0.4, 0.2, -0.6),
    );
    player.set_actor_placement(
        placement.w_axis.truncate().to_array(),
        glam::Quat::from_rotation_y(0.8).to_array(),
    )?;
    player.apply_actor_pose(&rest)?;
    for (x, y) in before.iter().zip(player.offhand_elements().unwrap()) {
        ensure!(
            placement
                .transform_point3(x.matrix.position.into())
                .distance(y.matrix.position.into())
                < 2e-5,
            "offhand escaped actor placement"
        );
    }
    player.set_actor_placement([0.; 3], [0., 0., 0., 1.])?;
    player.apply_actor_pose(&shifted)?;
    let world_origin = |pose| {
        glam::Mat4::from_cols_array(&assets::world_matrices(&rig, pose)[left])
            .w_axis
            .truncate()
    };
    let delta = world_origin(&shifted) - world_origin(&rest);
    ensure!(
        delta.length() > 0.05 && !before.is_empty(),
        "invalid left-hand fixture"
    );
    for (a, b) in before.iter().zip(player.offhand_elements().unwrap()) {
        ensure!(
            a.id == b.id
                && (glam::Vec3::from(b.matrix.position) - glam::Vec3::from(a.matrix.position))
                    .distance(delta)
                    < 1e-4,
            "ElementId {} did not follow the left-hand pose",
            a.id
        );
    }
    player.apply_actor_pose(&rest)?;
    let event: Event = serde_json::from_value(serde_json::json!({
        "chain":[{"tmb":"probe","actor":0,"track":0,"command":0,"actor_start":0,"track_start":0,"start":0,"duration":60,"ability_delay":0}],
        "avfx":"vfx/common/eff/cmcn_shoot1c.avfx","scale":[1,1,1],"rotation_radians":[0,0,0],"position":[0,0,0],"color":[1,1,1,1],"visibility":0,
        "targets":[{"origin":1,"kind":2,"id":-1},{"origin":2,"kind":0,"id":-1}]
    }))?;
    let config: Preview = serde_json::from_str(include_str!("../../examples/stone-preview.json"))?;
    let (action, draw) = ActionPlayer::load(vec![event], config, |p| Ok(game.read(p)?))?;
    player.play_action(action, draw);
    let replay = |p: &mut EffectPlayer| -> Result<Vec<u8>> {
        p.reset();
        p.refresh_equipped_action_weapons()?;
        for _ in 0..4 {
            frame(p, 0.1, true)?;
        }
        ensure!(
            p.action_player_mut().unwrap().active_occurrences().count() == 1,
            "spell not active"
        );
        pixels()
    };
    let full = replay(player)?;
    if let Some(dir) = std::env::var_os("OFFHAND_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        for (name, rgba) in [
            ("owner-hidden", &hidden),
            ("owner-visible", &dual),
            ("two-residents-action", &full),
        ] {
            let mut png = png::Encoder::new(
                std::fs::File::create(dir.join(format!("{name}.png")))?,
                256,
                256,
            );
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()?.write_image_data(rgba)?;
        }
    }
    let state = player.random_state();
    ensure!(
        full == replay(player)? && state == player.random_state(),
        "dual-weapon action replay diverged"
    );
    player.replay_with_random_seed([1, 2, 3, 4]);
    ensure!(
        player.random_state() == [1, 2, 3, 4],
        "scene seed not installed"
    );
    let alternate = replay(player)?;
    // Random draws need not change a visible fragment (e.g. occluded dust).
    // Verify the actual number of calls from two independent starting states.
    let mut original =
        assets::VfxClientRandomState::from_words([123456789, 362436069, 521288629, 88675123]);
    let mut alt = assets::VfxClientRandomState::from_words([1, 2, 3, 4]);
    let calls = (1..=1_000_000).find(|_| {
        original.next_u16();
        alt.next_u16();
        original.words() == state
    });
    ensure!(
        calls.is_some() && alt.words() == player.random_state(),
        "scene RNG consumption diverged between seeds"
    );
    ensure!(
        alternate == replay(player)?,
        "alternate seed replay diverged"
    );
    player.replay_with_random_seed([123456789, 362436069, 521288629, 88675123]);
    ensure!(
        full == replay(player)? && state == player.random_state(),
        "seed restoration diverged"
    );
    ensure!(
        player.offhand_elements().unwrap().iter().any(|p| p.id == 5),
        "saw ElementId missing"
    );
    let diag = player.diagnostics();
    ensure!(
        !diag
            .iter()
            .any(|s| s.starts_with("offhand ") && s.contains("overflow")),
        "{diag:?}"
    );
    player.clear_action();
    ensure!(
        state == player.random_state(),
        "disposing one document rewound the scene RNG"
    );
    frame(player, 0., true)?;
    let action_delta = diff(&full, &pixels()?);
    eprintln!(
        "offhand: secondary {visible} changed pixels; dual-saw action {action_delta}; replay identical; {} secondary ElementIds",
        pose.len()
    );
    ensure!(action_delta > 0, "spell did not affect HDR composite");
    player.clear_offhand();
    Ok(())
}
