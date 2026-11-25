use super::*;
use ::assets::{
    AvfxBinder, AvfxBinderData, AvfxBinderProperties, AvfxCurve, AvfxCurveKey, AvfxEmitter,
    AvfxEmitterItem, AvfxParticle, AvfxSchedulerItem, AvfxTimeline, AvfxTimelineItem, EmitterType,
    ParticleType, VfxCameraViewSnapshot,
};

const CASTER: BindTarget = BindTarget {
    origin: 1,
    kind: 0,
    id: -1,
};
const TARGET: BindTarget = BindTarget {
    origin: 2,
    kind: 0,
    id: -1,
};

fn binding(selector: BindTarget, object: f32, element: f32) -> Binding {
    Binding {
        target: selector,
        position: [object, 0., 0.],
        quaternion: identity_quaternion(),
        scale: unit_scale(),
        elements: vec![Element {
            id: 4,
            basis: VfxBinderMatrix::IDENTITY.basis,
            position: [element, 0., 0.],
        }],
    }
}
fn curve(z: f32) -> AvfxCurve {
    AvfxCurve {
        keys: vec![AvfxCurveKey {
            time: 0,
            interpolation: 1,
            x: 0.,
            y: 0.,
            z,
        }],
        ..Default::default()
    }
}
fn player() -> Result<ActionPlayer> {
    let cfg = Preview {
        controllers: vec![],
        units_per_second: 30.,
        crossfade_clips: false,
        bindings: vec![binding(CASTER, 0., 1.), binding(TARGET, 2., 3.)],
        enabled_tracks: vec![],
        condition_inputs: None,
        command_frames: vec![],
        directional_goals: vec![],
    };
    let event = Event {
        chain: vec![Link {
            tmb: "point.tmb".into(),
            actor: 0,
            track: 0,
            command: 1,
            actor_start: 0,
            track_start: 0,
            start: 0,
            duration: 30,
            ability_delay: 0,
            conditions: None,
        }],
        avfx: "point.avfx".into(),
        targets: [CASTER, TARGET],
        scale: unit_scale(),
        rotation_radians: [0.; 3],
        position: [0.; 3],
        color: [1.; 4],
        visibility: 0,
    };
    let (mut p, _) = ActionPlayer::load(vec![event], cfg, |_| Ok(b"XFVA\0\0\0\0".to_vec()))?;
    // One Document: three staged timeline Point binders query the caster
    // ElementId, target object origin and target ElementId respectively.
    let mut file = AvfxFile {
        emitters: vec![AvfxEmitter {
            emitter_type: Some(EmitterType::Point),
            effector_index: -1,
            create_count: curve(1.),
            create_interval: curve(100.),
            particle_items: vec![AvfxEmitterItem {
                enabled: true,
                create_count: 1,
                create_probability: 100,
                parameter_link: -1,
                parent_influence_coord: 2,
                ..Default::default()
            }],
            ..Default::default()
        }],
        particles: vec![AvfxParticle {
            particle_type: Some(ParticleType::Quad),
            collision_type: -1,
            ..Default::default()
        }],
        schedulers: vec![Default::default()],
        ..Default::default()
    };
    // A First/Always color channel makes redundant inputs observable through
    // the client stream even if all three positions remain unchanged.
    file.particles[0].color.random[0] = Some(AvfxCurve {
        random_type: 4,
        ..curve(0.25)
    });
    for (i, (obj, point)) in [(0, 3), (1, 0), (1, 3)].into_iter().enumerate() {
        file.binders.push(AvfxBinder {
            binder_type: 0,
            bind_point_id: 4,
            life: 30,
            transform_scale: 255,
            following_target_orientation: true,
            vfx_scale_bias: 1.,
            properties_start: Some(AvfxBinderProperties {
                bind_point_type: obj,
                bind_target_point_type: point,
                bct: if point == 0 { -1 } else { 0 },
                bind_point_id: 4,
                coord_update_frame: -1,
                ..Default::default()
            }),
            data: Some(AvfxBinderData {
                spring_strength: Some(curve(1.)),
                ..Default::default()
            }),
            ..Default::default()
        });
        file.timelines.push(AvfxTimeline {
            binder_index: i as i32,
            items: vec![AvfxTimelineItem {
                enabled: true,
                start_time: 0,
                end_time: 30,
                binder_index: -1,
                effector_index: -1,
                emitter_index: 0,
                platform: 0,
                clip_index: -1,
            }],
            ..Default::default()
        });
        file.schedulers[0].items.push(AvfxSchedulerItem {
            enabled: true,
            start_time: 0,
            timeline_index: i as i32,
        });
    }
    p.assets[0].materials = scene::ParticleMaterials::prepare(&mut file);
    p.assets[0].file = file;
    Ok(p)
}
fn positions(p: &ActionPlayer) -> Vec<f32> {
    let mut positions: Vec<_> = p.sample().0.into_iter().map(|q| q.position[0]).collect();
    positions.sort_by(f32::total_cmp);
    positions
}
fn camera(x: f32) -> VfxCameraViewSnapshot {
    let inv = glam::Mat4::from_translation(glam::Vec3::new(x, 0., 2.));
    VfxCameraViewSnapshot::from_view_matrices(
        inv.inverse().to_cols_array(),
        inv.to_cols_array(),
        256,
    )
}

#[test]
fn newborn_target_uses_latest_complete_valid_binding() -> Result<()> {
    let mut p = player()?;
    p.set_target_binding(binding(TARGET, 9., 8.))?;
    p.set_target_binding(binding(TARGET, 6., 7.))?;
    let mut invalid = binding(TARGET, 100., 9.);
    invalid
        .elements
        .push(binding(TARGET, 100., 10.).elements.remove(0));
    assert!(p.set_target_binding(invalid).is_err());
    let mut invalid = binding(TARGET, 10., 9.);
    invalid.quaternion[0] = f32::NAN;
    assert!(p.set_target_binding(invalid).is_err());
    p.advance_to(0.)?;
    assert_eq!(positions(&p), [1., 6., 7.]);
    Ok(())
}

#[test]
fn fallback_accepts_target_replacement_without_recreating_document() -> Result<()> {
    let mut p = player()?;
    // Deliberately unsupported Binder forces the pin's continuous path;
    // its explicit staged-only advance_with_* method would reject this input.
    p.assets[0].file.binders[0].binder_type = 255;
    p.set_camera_view(camera(0.))?;
    p.advance_to(0.)?;
    let reason = p.live[0]
        .playback
        .fallback_reason()
        .expect("fixture takes fallback")
        .to_owned();
    p.set_target_binding(binding(TARGET, 7., 11.))?;
    p.advance_to(0.)?;
    assert_eq!(p.live[0].target.object.unwrap().position[0], 2.);
    p.set_camera_view(camera(1.))?;
    p.set_caster_elements(CASTER, binding(CASTER, 0., 5.).snapshot()?.targets)?;
    p.advance_to(1. / 30.)?;
    let live = &p.live[0];
    assert_eq!(live.age, 1. / 30.);
    assert_eq!(live.playback.fallback_reason(), Some(reason.as_str()));
    assert_eq!(live.target.object.unwrap().position[0], 7.);
    assert_eq!(live.target.targets[0].matrix.position[0], 11.);
    Ok(())
}

#[test]
fn staged_point_targets_defer_at_equal_age_and_refresh_both_providers_once() -> Result<()> {
    for with_view in [true, false] {
        let mut p = player()?;
        if with_view {
            p.set_camera_view(camera(0.))?;
        }
        p.advance_to(0.)?;
        assert_eq!(p.live[0].playback.fallback_reason(), None);
        let initial = positions(&p);
        assert_eq!(initial, [1., 2., 3.]);
        let rng = p.live[0].playback.client_binder_random_state();
        p.set_target_binding(binding(TARGET, 7., 11.))?;
        p.advance_to(0.)?;
        assert_eq!(positions(&p), initial);
        assert_eq!(p.live[0].playback.client_binder_random_state(), rng);
        let caster = binding(CASTER, 0., 5.).snapshot()?.targets;
        p.set_caster_elements(CASTER, caster.clone())?;
        if with_view {
            p.set_camera_view(camera(1.))?;
        }
        p.advance_to(1. / 30.)?;
        assert_eq!(p.live[0].playback.fallback_reason(), None);
        let moved = positions(&p);
        assert_eq!(moved, [5., 7., 11.]);
        let rng = p.live[0].playback.client_binder_random_state();
        assert!(rng.is_some());
        for _ in 0..2 {
            p.set_caster_elements(CASTER, caster.clone())?;
            p.set_target_binding(binding(TARGET, 7., 11.))?;
            if with_view {
                p.set_camera_view(camera(1.))?;
            }
            p.advance_to(1. / 30.)?;
            assert_eq!(positions(&p), moved);
            assert_eq!(p.live[0].playback.client_binder_random_state(), rng);
        }
    }
    Ok(())
}
