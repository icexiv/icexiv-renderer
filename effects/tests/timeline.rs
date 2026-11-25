use anyhow::Result;
use icexiv_effects::timeline::{ActionPlayer, Event, Preview};
use serde_json::{Value, json};

fn link(path: &str, command: u16, start: i16, duration: i32, conditions: Value) -> Value {
    json!({"tmb":path,"actor":2,"track":3,"command":command,"actor_start":1,"track_start":0,"start":start,"duration":duration,"conditions":conditions,"ability_delay":0})
}
fn effect(chain: Vec<Value>, part: i8) -> Event {
    serde_json::from_value(json!({"chain":chain,"avfx":"shared.avfx","scale":[1,1,1],"rotation_radians":[0,0,0],"position":[0,0,0],"color":[1,1,1,1],"visibility":0,"targets":[{"origin":1,"kind":part,"id":-1},{"origin":2,"kind":0,"id":-1}]})).unwrap()
}
fn preview() -> Preview {
    serde_json::from_str(include_str!("../examples/stone-preview.json")).unwrap()
}

#[test]
fn host_trigger_requires_a_live_occurrence_and_resolves_a_one_based_slot() -> Result<()> {
    fn block(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
        tag.iter()
            .rev()
            .copied()
            .chain((data.len() as u32).to_le_bytes())
            .chain(data.iter().copied())
            .collect()
    }
    let item = [
        block(b"bEna", &1u32.to_le_bytes()),
        block(b"TlNo", &0u32.to_le_bytes()),
    ]
    .concat();
    let file = block(
        b"AVFX",
        &[
            block(
                b"Schd",
                &[
                    block(b"ItCn", &1i32.to_le_bytes()),
                    block(b"Item", &item),
                    block(b"Trgr", &item),
                ]
                .concat(),
            ),
            block(b"TmLn", &block(b"Loop", &(-1i32).to_le_bytes())),
        ]
        .concat(),
    );
    let mut p = ActionPlayer::load(
        vec![effect(vec![link("fx", 7, 0, 60, Value::Null)], 1)],
        preview(),
        |_| Ok(file.clone()),
    )?
    .0;
    assert!(!p.trigger(0, 0, 1)?);
    p.advance_to(1. / 30.)?;
    assert!(!p.trigger(1, 0, 1)?);
    assert!(p.trigger(0, 0, 0).is_err());
    assert!(p.trigger(0, 0, 1)?);
    assert!(p.fade(0, 0, 4, 2, false)?);
    p.advance_to(3.)?;
    assert!(!p.trigger(0, 0, 1)?);
    Ok(())
}
#[test]
fn repeated_child_calls_create_independent_documents_and_clip_at_parent_end() -> Result<()> {
    let parent = link("stone", 11, 0, 5, Value::Null);
    let child = link("end", 5, 0, 30, json!([[1, 5, 0]]));
    let events = vec![
        effect(vec![parent.clone(), child.clone()], 1),
        effect(vec![parent, child], 2),
    ];
    let mut config = preview();
    config.enabled_tracks = vec![("end".into(), 3)];
    let mut reads = 0;
    let (mut player, _) = ActionPlayer::load(events, config, |_| {
        reads += 1;
        Ok(b"XFVA\0\0\0\0".to_vec())
    })?;
    assert_eq!(reads, 1);
    player.advance_to(1. / 30.)?;
    assert_eq!(player.active_occurrences().count(), 0);
    player.advance_to(2. / 30.)?;
    assert_eq!(player.active_occurrences().collect::<Vec<_>>(), [0, 1]);
    player.advance_to(6. / 30.)?;
    assert_eq!(player.active_occurrences().count(), 0);
    assert!(player.advance_to(0.).is_err());
    player.reset();
    player.advance_to(2. / 30.)?;
    assert_eq!(player.active_occurrences().count(), 2);
    Ok(())
}
#[test]
fn skipped_conditions_and_missing_bindings_do_not_load_or_spawn() -> Result<()> {
    let events = vec![
        effect(vec![link("end", 5, 0, 30, json!([[1, 5, 0]]))], 1),
        effect(vec![link("stone", 13, 2, 30, Value::Null)], 9),
    ];
    let (mut p, _) = ActionPlayer::load(events, preview(), |_| panic!("unexpected resource read"))?;
    p.advance_to(0.5)?;
    assert_eq!(p.active_occurrences().count(), 0);
    assert!(p.diagnostics[0].contains("not activated"));
    assert!(p.diagnostics[1].contains("unresolved"));
    Ok(())
}
#[test]
fn nondefault_trs_on_nonidentity_host_requires_an_occurrence_specific_resolved_frame() -> Result<()>
{
    let make = || {
        let mut e = effect(vec![link("t", 1, 0, 3, Value::Null)], 1);
        e.position = [3., 0., 0.];
        e
    };
    let (p, _) = ActionPlayer::load(vec![make()], preview(), |_| {
        panic!("unsupported command loaded")
    })?;
    assert!(p.diagnostics[0].contains("resolved command frame"));
    let mut config = preview();
    config.command_frames.push(serde_json::from_value(json!({"occurrence":0,"position":[4,0,0],"basis":[[1,0,0],[0,1,0],[0,0,1]],"scale":[1,1,1]}))?);
    let (mut p, _) = ActionPlayer::load(vec![make(), make()], config, |_| {
        Ok(b"XFVA\0\0\0\0".to_vec())
    })?;
    p.advance_to(1. / 30.)?;
    assert_eq!(p.active_occurrences().collect::<Vec<_>>(), [0]);
    Ok(())
}

#[test]
fn c012_affine_order_and_visibility_admission() -> Result<()> {
    let make = |flags| {
        let mut e = effect(vec![link("t", 10, 0, 30, Value::Null)], 0);
        e.scale = [2., -3., 4.];
        e.rotation_radians = [0.31, -0.72, 1.12];
        e.position = [0., -1.7, 1.25];
        e.visibility = flags;
        e
    };
    // Independent vector rotations: rotate each scaled basis vector about
    // world X, then Y, then Z. Mixed axes/negative scale expose transposes.
    let m = make(0).command_matrix();
    let expected = [
        [0.6550971, 1.3534013, 1.3187693],
        [2.8345015, -0.7015768, -0.6880345],
        [0.00397924, -2.792522, 2.8638794],
    ];
    for (a, b) in m
        .basis
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!((a - b).abs() < 1e-6);
    }
    assert_eq!(m.position, [0., -1.7, 1.25]);
    let (mut p, _) =
        ActionPlayer::load(vec![make(0), make(2), make(1), make(4)], preview(), |_| {
            Ok(b"XFVA\0\0\0\0".to_vec())
        })?;
    p.advance_to(1. / 30.)?;
    assert_eq!(p.active_occurrences().collect::<Vec<_>>(), [0, 1, 2, 3]);
    Ok(())
}

#[test]
fn named_clips_share_child_windows_and_refuse_ambiguous_overlap() -> Result<()> {
    use icexiv_effects::timeline::{NamedClip, clip_windows};
    let clip = |name, chain: Vec<Value>| -> NamedClip {
        serde_json::from_value(json!({"name":name,"chain":chain})).unwrap()
    };
    let parent = link("parent", 11, 0, 5, Value::Null);
    let child = link("child", 4, 1, 30, Value::Null);
    let (w, notes) = clip_windows(
        vec![
            clip("idle", vec![parent.clone(), child.clone()]),
            clip("off", vec![link("skip", 3, 0, 8, json!([[1, 2, 0]]))]),
        ],
        &preview(),
    )?;
    assert_eq!(
        (w.len(), w[0].name.as_str(), w[0].start, w[0].end),
        (1, "idle", 3. / 30., 6. / 30.)
    );
    assert_eq!(notes.len(), 1);
    assert!(
        clip_windows(
            vec![clip("a", vec![child.clone()]), clip("b", vec![child])],
            &preview()
        )
        .is_err()
    );
    let mut cfg = preview();
    cfg.crossfade_clips = true;
    let window = |s, d| clip("same", vec![link("chain", 4, s, d, Value::Null)]);
    assert!(clip_windows(vec![window(0, 12), window(11, 29)], &cfg).is_ok());
    assert!(clip_windows(vec![window(0, 12), window(10, 16), window(11, 28)], &cfg).is_err());
    assert!(clip_windows(vec![window(0, 12), window(2, 3)], &cfg).is_err());
    Ok(())
}

#[test]
#[ignore = "requires extracted Stone TMB/AVFX/ATEX tree and --clips manifest"]
fn stone_live_element_and_camera() -> Result<()> {
    use icexiv_effects::timeline::BindTarget;
    let root = std::path::PathBuf::from(std::env::var("ACTION_ROOT")?);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(std::env::var("STONE_MANIFEST")?)?)?;
    assert_eq!(manifest["clips"][0]["name"], "cbbm_mgc_cnj_3b");
    let sample = |moved: bool| -> Result<Vec<[f32; 3]>> {
        let (mut p, _) = ActionPlayer::load(
            serde_json::from_value(vec![manifest["effects"][0].clone()].into())?,
            preview(),
            |path| Ok(std::fs::read(root.join(path))?),
        )?;
        let mut camera = assets::VfxCameraViewSnapshot::from_view_matrices(
            glam::Mat4::IDENTITY.to_cols_array(),
            glam::Mat4::IDENTITY.to_cols_array(),
            256,
        );
        for tick in 0..=30 {
            if tick == 12 {
                camera.screen_height = 512;
                if moved {
                    p.set_caster_elements(
                        BindTarget {
                            origin: 1,
                            kind: 1,
                            id: -1,
                        },
                        vec![assets::VfxBinderTargetSnapshot {
                            id: 5,
                            matrix: assets::VfxBinderMatrix {
                                basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                                position: [7., 0.9, 0.],
                            },
                        }],
                    )?;
                }
            }
            p.set_camera_view(camera)?;
            p.advance_to(tick as f64 / 60.)?;
            let (q, _) = p.sample();
            if tick == 30 {
                return Ok(q.iter().map(|v| v.position).collect());
            }
        }
        unreachable!()
    };
    let (fixed, moved) = (sample(false)?, sample(true)?);
    assert!(
        fixed.iter().all(|v| v[0] < 3.) && moved.iter().any(|v| v[0] > 3.),
        "moving ElementId 5 should move the live weapon effect"
    );
    Ok(())
}

#[test]
#[ignore = "requires extracted original Stone AVFX and C012 manifest"]
fn stone_resolved_directional_goal_is_not_the_target_origin() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::var("ACTION_ROOT")?);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(std::env::var("STONE_MANIFEST")?)?)?;
    let cue = manifest["effects"][2].clone();
    assert!(cue["avfx"].as_str().unwrap().ends_with("mgc_ston1t0h.avfx"));
    let goal = |x| {
        json!({"occurrence":0,"binder":0,"ordinal":0,
        "position":[x,8,-2],"basis":[[0,0,-2],[0,3,0],[4,0,0]]})
    };
    let failed = || json!({"occurrence":0,"binder":0,"ordinal":0,"missing":true});
    let sample =
        |initial: Option<Value>, change: Option<Value>, target_x: f32| -> Result<Vec<[f32; 3]>> {
            let mut cfg: Value =
                serde_json::from_str(include_str!("../examples/stone-preview.json"))?;
            cfg["bindings"][0]["elements"] =
                json!([{"id":16,"basis":[[1,0,0],[0,1,0],[0,0,1]],"position":[0,0.9,1]}]);
            cfg["bindings"][3]["position"] = json!([target_x, 0, 0]);
            let adapt = initial.is_some();
            if let Some(initial) = initial {
                cfg["directional_goals"] = json!([initial]);
            }
            let (mut p, _) = ActionPlayer::load(
                serde_json::from_value(json!([cue]))?,
                serde_json::from_value(cfg)?,
                |path| Ok(std::fs::read(root.join(path))?),
            )?;
            let mut positions = vec![];
            for tick in 0..65 {
                if tick == 18
                    && let Some(value) = &change
                {
                    p.set_directional_goal(serde_json::from_value(value.clone())?)?;
                }
                p.advance_to(tick as f64 / 60.)?;
                let (q, m) = p.sample();
                positions.extend(q.iter().map(|q| q.position));
                positions.extend(m.iter().map(|m| m.position));
            }
            assert_eq!(
                p.diagnostics
                    .iter()
                    .any(|s| s.contains("missing resolved Linear")),
                !adapt
            );
            assert_eq!(
                p.diagnostics.iter().any(|s| s.contains("staged playback:")),
                !adapt
            );
            Ok(positions)
        };
    sample(None, None, 0.)?;
    let a = sample(Some(goal(8.)), None, 0.)?;
    assert!(!a.is_empty());
    assert_eq!(a, sample(Some(goal(8.)), None, 99.)?);
    assert_ne!(a, sample(Some(goal(8.)), Some(goal(24.)), 0.)?);
    let missed = sample(Some(failed()), None, 99.)?;
    assert_ne!(a, missed);
    // This Stone scheduler has no second birth opportunity. A late success
    // must not retroactively create the missed projectile.
    assert!(missed.is_empty());
    assert_eq!(missed, sample(Some(failed()), Some(goal(8.)), 99.)?);
    // Failed refresh leaves the previously successful target cache intact.
    assert_eq!(a, sample(Some(goal(8.)), Some(failed()), 99.)?);
    Ok(())
}
