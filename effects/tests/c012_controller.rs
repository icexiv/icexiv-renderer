use anyhow::Result;
use icexiv_effects::timeline::{
    ActionPlayer, ControllerInput, ControllerPhase as Phase, Event, Preview,
};
use serde_json::json;

fn event(flag: u32) -> Event {
    serde_json::from_value(json!({
        "avfx":"vfx/common/eff/cmab_shoot1c.avfx",
        "chain":[{"ability_delay":0,"actor":2,"actor_start":1,"command":13,"conditions":null,"duration":30,"start":0,
            "tmb":"chara/action/ability/2gn_machin/abl001.tmb","track":5,"track_start":0}],
        "scale":[1,1,1],"rotation_radians":[0,0,0],"position":[0,0,0],"color":[1,1,1,1],
        "targets":[{"origin":1,"kind":0,"id":-1},{"origin":2,"kind":0,"id":-1}],"visibility":flag
    })).unwrap()
}
fn preview() -> Preview {
    serde_json::from_str(include_str!("../examples/stone-preview.json")).unwrap()
}
fn trigger_document() -> Vec<u8> {
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
    block(
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
    )
}
#[test]
fn controller_flags_evaluate_separate_queries_for_all_eight_combinations() -> Result<()> {
    for eligible in [None, Some(false), Some(true)] {
        for context_code in [0, 198, 199] {
            for state_32 in [false, true] {
                for allocation_available in [false, true] {
                    let mut cfg = preview();
                    cfg.controllers = (0..8)
                        .map(|occurrence| ControllerInput {
                            occurrence,
                            eligible,
                            context_code,
                            state_32,
                            allocation_available,
                            ..Default::default()
                        })
                        .collect();
                    let (mut p, _) = ActionPlayer::load((0..8).map(event).collect(), cfg, |_| {
                        Ok(trigger_document())
                    })?;
                    p.advance_to(1. / 30.)?;
                    for flag in 0..8 {
                        let s = p.controller_status(flag).unwrap();
                        assert_eq!(s.external, flag & 1 != 0);
                        assert_eq!(
                            s.phase,
                            if flag & 1 != 0 && !allocation_available {
                                Phase::AllocationFailed
                            } else if flag & 4 != 0 && state_32 {
                                Phase::StateBlocked
                            } else if flag & 2 == 0
                                && eligible == Some(false)
                                && context_code != 198
                            {
                                Phase::Ineligible
                            } else {
                                Phase::Attached
                            }
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn detached_controller_queues_one_trigger_and_outlives_its_command() -> Result<()> {
    let mut cfg = preview();
    cfg.controllers = vec![ControllerInput {
        occurrence: 0,
        manager_ready: false,
        ..Default::default()
    }];
    let (mut p, _) = ActionPlayer::load(vec![event(7), event(4), event(1)], cfg, |_| {
        Ok(trigger_document())
    })?;
    for _ in 0..2 {
        p.advance_to(0.1)?;
        for occurrence in [0, 1] {
            p.set_controller_input(ControllerInput {
                occurrence,
                state_32: true,
                manager_ready: false,
                ..Default::default()
            })?;
        }
        p.advance_to(0.1)?;
        assert_eq!(p.active_occurrences().collect::<Vec<_>>(), [2]);
        for occurrence in [0, 1] {
            let s = p.controller_status(occurrence).unwrap();
            assert!(s.state_32_stop && s.trigger_attached);
            assert_eq!(s.trigger_requests, 1);
            assert_eq!(
                s.phase,
                if occurrence == 0 {
                    Phase::Detached
                } else {
                    Phase::Released
                }
            );
        }
        assert!(
            !p.trigger(0, 0, 1)?,
            "command no longer owns the detached controller"
        );
        p.end_commands();
        p.end_commands();
        assert_eq!(p.controller_status(2).unwrap().phase, Phase::Detached);
        assert_eq!(p.controller_status(2).unwrap().trigger_requests, 1);
        p.advance_to(2.)?;
        assert_eq!(p.controller_status(2).unwrap().phase, Phase::Released);
        assert_eq!(p.controller_status(0).unwrap().phase, Phase::Detached);
        assert!(!p.finished());
        p.set_controller_input(ControllerInput::default())?;
        p.advance_to(2.01)?;
        assert!(p.finished());
        p.reset();
        assert_eq!(p.controller_status(0).unwrap().phase, Phase::Waiting);
    }
    Ok(())
}

#[test]
#[ignore = "reads original row 3829 TMB and AVFX from XIV_GAME_DIR"]
fn retail_external_shoot_command_renders_during_its_window_and_detaches() -> Result<()> {
    use physis::resource::{Resource, SqPackResource};
    use sha2::{Digest, Sha256};
    let mut game = SqPackResource::from_existing(&std::env::var("XIV_GAME_DIR")?);
    let tmb = game.read("chara/action/ability/2gn_machin/abl001.tmb")?;
    assert_eq!(
        format!("{:x}", Sha256::digest(&tmb)),
        "5646c300fb867363d621d746f8a9393ddd03c8db312c21a87c0d9574997f7ecb"
    );
    let u32at = |at: usize| u32::from_le_bytes(tmb[at..at + 4].try_into().unwrap());
    let mut at = 12;
    let mut actual = None;
    for _ in 0..u32at(8) {
        let p = at + 8;
        if &tmb[at..at + 4] == b"C012" && tmb[p..p + 2] == 13u16.to_le_bytes() {
            let path = p + u32at(p + 12) as usize;
            assert!(tmb[path..].starts_with(b"vfx/common/eff/cmab_shoot1c.avfx\0"));
            actual = Some(u32at(p + 56));
            break;
        }
        at += u32at(at + 4) as usize;
    }
    assert_eq!(actual, Some(1));
    // Same real AVFX twice, controlled mask-4 input on a live instance.
    // Compare the packet outputs, not just the controller status: teardown
    // destroys the embedded instance but leaves the external Document drawable.
    for (flag, draws_after_stop) in [(4, false), (5, true)] {
        let (mut p, _) =
            ActionPlayer::load(vec![event(flag)], preview(), |path| Ok(game.read(path)?))?;
        p.advance_to(1. / 30.)?;
        p.set_controller_input(ControllerInput {
            state_32: true,
            manager_ready: false,
            ..Default::default()
        })?;
        p.advance_to(1. / 30.)?;
        let (quads, meshes) = p.sample();
        assert_eq!(!quads.is_empty() || !meshes.is_empty(), draws_after_stop);
        assert!(p.active_occurrences().next().is_none());
    }
    let mut cfg = preview();
    cfg.controllers.push(ControllerInput {
        manager_ready: false,
        ..Default::default()
    });
    let (mut p, _) = ActionPlayer::load(vec![event(actual.unwrap())], cfg, |path| {
        Ok(game.read(path)?)
    })?;
    for _ in 0..2 {
        p.advance_to(1. / 30.)?;
        let (qs, ms) = p.sample();
        assert!(
            !qs.is_empty() || !ms.is_empty(),
            "retail AVFX did not emit geometry"
        );
        p.advance_to(31. / 30.)?;
        assert_eq!(p.controller_status(0).unwrap().phase, Phase::Detached);
        assert!(p.active_occurrences().next().is_none() && !p.finished());
        p.advance_to(32. / 30.)?;
        p.set_controller_input(ControllerInput::default())?;
        p.advance_to(33. / 30.)?;
        assert!(p.finished() && p.sample() == (vec![], vec![]));
        p.reset();
    }
    Ok(())
}
