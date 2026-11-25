#![cfg(all(feature = "gpu-tests", not(target_arch = "wasm32")))]

use assets::{ParticleType, VFX_IDENTITY_BASIS, VfxQuad};
use scene::test_support::{WeaponModelSnapshotOptions, render_model_snapshot_with_options};

fn quad(x: f32) -> VfxQuad {
    VfxQuad {
        particle_type: Some(ParticleType::Quad),
        particle_index: 0,
        soft_particle: false,
        soft_particle_fade_range: 1.,
        depth_offset_type: 0,
        depth_offset: 0.,
        powder_single: false,
        windmill_uv_type: 0,
        disc: None,
        polygon: None,
        laser: None,
        line: None,
        polyline: None,
        decal: None,
        position: [x, 0.5, 0.],
        size: [0.3; 2],
        orientation: [0., 0., 0., 1.],
        parent_basis: VFX_IDENTITY_BASIS,
        movement_direction: [0., 0., 1.],
        facing_parent_basis: VFX_IDENTITY_BASIS,
        rotation_direction_base: assets::avfx::rotation_direction_base::NONE,
        color: [0.25, 0.5, 2., 0.375],
        draw_layer: 0,
        soft_key_offset: 0.,
        draw_priority: 0,
        draw_order: None,
        pivot: [0.; 2],
        texture_indexes: [-1; 4],
        texture_uv_sets: [0; 4],
        combine_mode_tc1: [0; 2],
        combine_modes: [[0; 2]; 3],
        color_to_alpha: [false; 4],
        uv_origins: [[0.; 2]; 4],
        uv_scales: [[1.; 2]; 4],
        uv_by_pixel_position: [false; 4],
        uv_rotations: [0.; 4],
        texture_borders: [[0; 2]; 4],
        texture_filters: [0; 4],
        texture1_is_shape_mask: false,
        texture1_enabled: false,
        texture1_use_screen_copy: false,
        draw_mode: assets::DRAW_MODE_OPACITY,
        depth_test: false,
        depth_write: false,
        cull_mode: 0,
        texture_distortion_index: -1,
        distortion_power: 0.,
        distortion_targets: 0,
        uvd_origin: [0.; 2],
        uvd_scale: [1.; 2],
        uvd_rotation: 0.,
        uvd_by_pixel_position: false,
        distortion_uv_set: 0,
        distortion_borders: [0; 2],
        distortion_filter: 0,
        texture_palette_index: -1,
        palette_offset: 0.,
        palette_border: 0,
        palette_filter: 0,
    }
}

#[test]
#[ignore = "requires native GPU (XIV_ALLOW_GPU_TESTS=1)"]
fn forward_tone_preserves_alpha_and_separates_particle_definitions() {
    // A degenerate owner triangle defines the camera volume but shades no pixels.
    let owner = scene::ModelData {
        bounds: Default::default(),
        textures: vec![],
        materials: vec![],
        meshes: vec![scene::ModelMesh {
            path: "tone-test".into(),
            part_index: 0,
            mesh_category: None,
            submesh: None,
            shape_influences: vec![],
            shape_targets: vec![],
            material_index: 0,
            material_slot: 0,
            material_name: String::new(),
            color: [1.; 3],
            bone_table: None,
            vertices: [-1., 0., 1.]
                .map(|x| scene::ModelVertex {
                    position: [x, 0., 0.],
                    normal: [0., 0., 1.],
                    blend_weights: None,
                    blend_indices: None,
                    uv0: [0.; 2],
                    uv1: [0.; 2],
                    uv2: [0.; 2],
                    uv3: [0.; 2],
                    bitangent: [0., 1., 0., 1.],
                    normal1: None,
                    bitangent1: None,
                    color: [1.; 4],
                    color1: None,
                    flow0: None,
                    flow1: None,
                })
                .to_vec(),
            indices: vec![0, 1, 2],
        }],
    };
    // Two LightModel definitions deliberately share one model ID. Carry the
    // definition gate through real playback, then place the packets for readback.
    let model = assets::VfxDrawModel {
        vertices: [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]]
            .map(|[x, y]| assets::VfxDrawVertex {
                position: [x, y, 0.],
                position_w: 1.,
                normal: [128, 128, 255, 128],
                tangent: [255, 128, 128, 255],
                uvs: [[0.; 2]; 4],
                color: [255; 4],
            })
            .to_vec(),
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let mut file = assets::AvfxFile {
        timelines: vec![assets::AvfxTimeline {
            binder_index: -1,
            items: vec![assets::AvfxTimelineItem {
                enabled: true,
                start_time: 0,
                end_time: -1,
                binder_index: -1,
                effector_index: -1,
                emitter_index: 0,
                platform: 0,
                clip_index: -1,
            }],
            ..Default::default()
        }],
        emitters: vec![assets::AvfxEmitter {
            emitter_type: Some(assets::EmitterType::Point),
            create_count: assets::AvfxCurve {
                keys: vec![assets::AvfxCurveKey {
                    time: 0,
                    interpolation: 1,
                    x: 0.,
                    y: 0.,
                    z: 1.,
                }],
                ..Default::default()
            },
            particle_items: (0..2)
                .map(|i| assets::AvfxEmitterItem {
                    enabled: true,
                    target_index: i,
                    create_count: 1,
                    create_probability: 100,
                    parameter_link: -1,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
        models: vec![assets::VfxModelGeometry {
            draw: Some(model.clone()),
            ..Default::default()
        }],
        particles: [true, false]
            .map(|gate| assets::AvfxParticle {
                particle_type: Some(ParticleType::LightModel),
                data: assets::AvfxParticleData::LightModel { model_index: 0 },
                is_apply_tone_map: gate,
                depth_offset_type: if gate { 0 } else { 1 },
                draw_mode: assets::DRAW_MODE_OPACITY,
                collision_type: -1,
                ..Default::default()
            })
            .to_vec(),
        ..Default::default()
    };
    let materials = scene::ParticleMaterials::prepare(&mut file);
    let mut meshes = vec![];
    assets::VfxPlayback::new(assets::VfxRuntime::new(&file)).sample(&mut vec![], &mut meshes);
    assert_eq!(meshes.len(), 2);
    let gates: Vec<_> = meshes
        .iter_mut()
        .map(|m| {
            let gate = materials.mesh(m);
            assert_eq!(gate, m.depth_offset_type == 0);
            assert_eq!(m.model_index, 0);
            let q = quad(if gate { -0.5 } else { 0.5 });
            m.position = [q.position[0], -0.5, 0.];
            m.orientation = q.orientation;
            m.parent_basis = q.parent_basis;
            m.rotation_direction_base = q.rotation_direction_base;
            m.scale = [0.3, 0.3, 1.];
            m.color = q.color;
            m.fresnel = None;
            m.texture1_enabled = false;
            gate
        })
        .collect();
    let render = |name: &str, particle_tone: [f32; 3]| {
        let opts = WeaponModelSnapshotOptions {
            vfx_quad_tone: vec![true, false],
            vfx_mesh_tone: gates.clone(),
            ..WeaponModelSnapshotOptions::new(name)
                .with_camera(0., 0., 3., [0.; 2])
                .with_viewport(64, 64)
                .with_vfx_quads([quad(-0.5), quad(0.5)])
                .with_vfx_mesh_instances(meshes.iter().copied())
                .with_vfx_meshes([model.clone()])
                .with_render_options(scene::ModelRenderOptions {
                    particle_tone,
                    ..Default::default()
                })
                .with_hdr_scene_capture()
        };
        render_model_snapshot_with_options(opts, &owner)
            .unwrap()
            .hdr_scene_rgba
            .unwrap()
    };
    let neutral = render("bATM-neutral", [0., 1., 1.]);
    // T.w applies to both particles, independent of the reciprocal sqrt.
    let scaled = render("bATM-scaled", [0., 2., 1.]);
    let gated = render("bATM-gated", [0.5, 2., 7.]); // K = rsqrt(4) = 0.5
    let mut counts = [0; 4];
    for (i, ((a, b), c)) in neutral.iter().zip(&scaled).zip(&gated).enumerate() {
        if *a != [0.25, 0.5, 2., 0.375] {
            assert_eq!((a, a), (b, c), "background {i}");
            continue;
        }
        let side = usize::from(i % 64 >= 32);
        counts[side + 2 * usize::from(i / 64 >= 32)] += 1;
        assert_eq!(*b, [0.5, 1., 4., 0.375], "global scale {i}");
        assert_eq!(c, if side == 0 { a } else { b }, "gate / alpha {i}");
    }
    assert!(
        counts.iter().all(|n| *n > 16),
        "particle coverage {counts:?}"
    );
    eprintln!("HDR pixels checked per particle: {counts:?}");

    // The Disc vertex shader clamps endpoint colors to 10 before interpolation.
    // 12 * 0.5 must therefore read back as 5, not 6. Neither the ordinary
    // quad nor the mesh shares that pack, despite sharing the Document factor.
    let mut disc = quad(-0.5);
    disc.particle_type = Some(ParticleType::Disc);
    disc.orientation = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2).to_array();
    disc.color[0] = 12.;
    disc.disc = Some(assets::avfx_sim::VfxDisc {
        counts: [1, 2, 24],
        angle: std::f32::consts::TAU,
        radius: [0.3; 2],
        width: [0.2; 2],
        height_inner: [0.; 2],
        height_outer: [0.; 2],
        color_inner: [1.; 4],
        color_outer: [1.; 4],
        point_interval_factor: 0.,
        scaling_scale: 1.,
        scale_z: 0.6,
    });
    let factor = [0.5, 2., 0., 0.5];
    let opts = WeaponModelSnapshotOptions {
        vfx_quad_document_colors: vec![factor; 2],
        vfx_mesh_document_colors: vec![factor; 2],
        ..WeaponModelSnapshotOptions::new("C012-Disc-Quad-LightModel")
            .with_camera(0., 0., 3., [0.; 2])
            .with_viewport(64, 64)
            .with_vfx_quads([disc, quad(0.5)])
            .with_vfx_mesh_instances(meshes)
            .with_vfx_meshes([model])
            .with_hdr_scene_capture()
    };
    let hdr = render_model_snapshot_with_options(opts, &owner)
        .unwrap()
        .hdr_scene_rgba
        .unwrap();
    let mut coverage = [0; 4];
    for (i, pixel) in hdr.iter().enumerate() {
        let quadrant = usize::from(i % 64 >= 32) + 2 * usize::from(i / 64 >= 32);
        let red = if quadrant == 0 { 5. } else { 0.125 };
        if *pixel == [red, 1., 0., 0.1875] {
            coverage[quadrant] += 1;
        }
    }
    assert!(
        coverage.iter().all(|n| *n > 16),
        "Document HDR pixels: {coverage:?}"
    );
}
