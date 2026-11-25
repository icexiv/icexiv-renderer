use assets::{
    AvfxFile, VfxClientBinderCurveDefaults, VfxClientColorCurveDefaults, VfxClientRandomState,
    VfxClientTrigMode, VfxRuntime, VfxSharedRandomStream,
};

/// Action documents on one preview thread consume one ordered RNG stream.
/// Each occurrence still owns its local ages and its cached constructor draws.
pub(crate) struct ParticleEnvironment(pub(crate) std::sync::Arc<VfxSharedRandomStream>, bool);

impl Default for ParticleEnvironment {
    fn default() -> Self {
        // Same reproducible preview seed as scene's WeaponVfxPlayback. Live
        // client TLS state is not stored in the AVFX.
        Self::seeded([123456789, 362436069, 521288629, 88675123])
    }
}

impl ParticleEnvironment {
    pub(crate) fn seeded(words: [u32; 4]) -> Self {
        Self(
            std::sync::Arc::new(VfxSharedRandomStream::new(
                VfxClientRandomState::from_words(words),
            )),
            true,
        )
    }
    pub(crate) fn borrowed(&self) -> Self {
        Self(self.0.clone(), false)
    }
    pub(crate) fn reset(&self) {
        if self.1 {
            self.0.reset();
        }
    }

    pub(crate) fn runtime(&self, file: &AvfxFile) -> VfxRuntime {
        // Pinned icexiv-data's Col compiler and common particle constructors
        // implement retained First draws and per-property Always draws. A bare
        // VfxRuntime instead takes the seeded preview path for those channels.
        let mut runtime = VfxRuntime::new(file)
            .with_shared_client_random_stream(&self.0, VfxClientTrigMode::Sse2)
            .with_client_color_curve_defaults(VfxClientColorCurveDefaults {
                empty_random_type: 0,
                empty_rgba: [1.; 4],
            })
            .with_client_particle_constructor();
        if file.binders.iter().any(|binder| binder.binder_type == 2) {
            runtime = runtime.with_client_binder_curve_defaults(VfxClientBinderCurveDefaults {
                empty_scalar_random_type: 0,
                vector_random_fallback: [0.; 3],
            });
        }
        runtime
    }
}

/// Convert the consumed playback copy once. The scene's forward-Z projection
/// adds DpOf to clip Z for both DOTy modes, moving positive offsets away.
/// Retain signed offsets, binder multipliers, depth test and write behavior.
pub(crate) fn prepare_forward_depth(file: &mut AvfxFile) {
    for particle in &mut file.particles {
        particle.depth_offset = -particle.depth_offset;
    }
}

#[derive(Default)]
pub(crate) struct PacketNamespace {
    order: u64,
    particle: usize,
}

impl PacketNamespace {
    /// One ordering namespace for both mesh and quad draws. Preserve the gap
    /// between documents so GPU runs cannot merge across the boundary.
    pub(crate) fn rebase(
        &mut self,
        quads: &mut [assets::VfxQuad],
        meshes: &mut [assets::VfxMeshInstance],
        definition_count: usize,
    ) {
        for q in &mut *quads {
            q.particle_index += self.particle;
        }
        self.particle += definition_count;
        let mut next = self.order;
        for order in quads
            .iter_mut()
            .map(|q| &mut q.draw_order)
            .chain(meshes.iter_mut().map(|m| &mut m.draw_order))
            .flatten()
        {
            *order = order.saturating_add(self.order);
            next = next.max(order.saturating_add(2));
        }
        self.order = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assets::{
        AvfxColorCurve, AvfxCurve, AvfxCurveKey, AvfxEmitter, AvfxEmitterItem, AvfxParticle,
        AvfxTimeline, AvfxTimelineItem, EmitterType, ParticleType, VfxPlayback,
    };

    fn key(time: i16, interpolation: u16, [x, y, z]: [f32; 3]) -> AvfxCurveKey {
        AvfxCurveKey {
            time,
            interpolation,
            x,
            y,
            z,
        }
    }
    fn curve(keys: Vec<AvfxCurveKey>) -> AvfxCurve {
        AvfxCurve {
            keys,
            ..Default::default()
        }
    }
    fn one(z: f32) -> AvfxCurve {
        curve(vec![key(0, 1, [0., 0., z])])
    }

    fn document() -> AvfxFile {
        let mut particle = AvfxParticle {
            particle_type: Some(ParticleType::Quad),
            collision_type: -1,
            depth_offset: 0.125,
            depth_test: true,
            draw_mode: assets::DRAW_MODE_ADD,
            color: AvfxColorCurve {
                rgb: Some(curve(vec![
                    key(0, 2, [1., 1., 0.]),
                    key(4, 0, [1., 1., 2.]),
                ])),
                alpha: Some(curve(vec![
                    key(0, 1, [0., 0., 1.]),
                    key(4, 2, [0., 0., 0.5]),
                ])),
                brightness: Some(one(2.)),
                ..Default::default()
            },
            ..Default::default()
        };
        particle.color.random[0] = Some(AvfxCurve {
            random_type: 1,
            ..one(0.5)
        });
        AvfxFile {
            timelines: vec![AvfxTimeline {
                binder_index: -1,
                items: vec![AvfxTimelineItem {
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
            emitters: vec![AvfxEmitter {
                emitter_type: Some(EmitterType::Point),
                effector_index: -1,
                create_count: one(1.),
                create_interval: one(1000.),
                particle_items: vec![AvfxEmitterItem {
                    enabled: true,
                    create_probability: 100,
                    create_count: 1,
                    parameter_link: -1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            particles: vec![particle],
            ..Default::default()
        }
    }

    #[test]
    fn ordered_documents_keep_first_color_draw_and_replay_hdr_and_step_alpha() {
        let mut file = document();
        prepare_forward_depth(&mut file);
        let environment = ParticleEnvironment::default();
        let run = || {
            let mut doc = VfxPlayback::new(environment.runtime(&file));
            let mut samples = Vec::new();
            for step in [0., 2., 2., 2.] {
                doc.advance(step / assets::AVFX_FPS).unwrap();
                assert_eq!(doc.fallback_reason(), None);
                let mut quads = Vec::new();
                doc.sample(&mut quads, &mut Vec::new());
                let [q] = quads.as_slice() else {
                    panic!("expected one quad")
                };
                assert!(q.depth_test && !q.depth_write);
                assert_eq!(q.depth_offset, -0.125);
                samples.push(q.color);
            }
            samples
        };
        let first = run();
        // Staged drawing consumes cached properties from the Time callback:
        // the first positive input submits age zero, then ages two and four.
        assert_eq!(first[1], first[0]);
        assert_eq!(first[2], [first[0][0], 2., 2., 1.]);
        assert_eq!(first[3], [first[0][0], 2., 4., 0.5]);
        let second = run();
        assert_ne!(first[0][0], second[0][0]);
        environment.reset();
        assert_eq!(first, run());
        assert_eq!(second, run());
    }

    #[test]
    fn double_sided_definitions_and_order_ranges_do_not_alias_across_documents() {
        let mut file = document();
        file.particles[0].culling_type = 3;
        let mut model = file.particles[0].clone();
        model.particle_type = Some(ParticleType::LightModel);
        model.data = assets::AvfxParticleData::LightModel { model_index: 0 };
        file.models.push(assets::VfxModelGeometry {
            draw: Some(Default::default()),
            ..Default::default()
        });
        file.particles.push(model);
        let model_item = AvfxEmitterItem {
            target_index: 1,
            ..file.emitters[0].particle_items[0]
        };
        file.emitters[0].particle_items.push(model_item);
        let environment = ParticleEnvironment::default();
        let mut namespace = PacketNamespace::default();
        let mut ranges = vec![];
        for _ in 0..2 {
            let doc = VfxPlayback::new(environment.runtime(&file));
            let (mut qs, mut ms) = (vec![], vec![]);
            doc.sample(&mut qs, &mut ms);
            assert_eq!((qs.len(), ms.len()), (1, 1));
            namespace.rebase(&mut qs, &mut ms, file.particles.len());
            ranges.push((
                qs[0].particle_index,
                qs[0].draw_order.unwrap(),
                ms[0].draw_order.unwrap(),
            ));
        }
        let [(def_a, q_a, m_a), (def_b, q_b, m_b)] = ranges[..] else {
            unreachable!()
        };
        assert_ne!(def_a, def_b);
        assert_eq!(q_a < m_a, q_b < m_b);
        assert!(q_a.max(m_a) + 1 < q_b.min(m_b));
    }

    #[test]
    fn mesh_material_provenance_restores_different_raw_doty_on_one_geometry() {
        let mut authored = document();
        authored.models.push(assets::VfxModelGeometry {
            draw: Some(Default::default()),
            ..Default::default()
        });
        for (raw, enabled) in [(i32::MAX, true), (-13, false)] {
            let mut p = authored.particles[0].clone();
            p.particle_type = Some(ParticleType::LightModel);
            p.data = assets::AvfxParticleData::LightModel { model_index: 0 };
            p.depth_offset_type = raw;
            p.is_apply_tone_map = enabled;
            let target_index = authored.particles.len() as i32;
            authored.particles.push(p);
            let item = AvfxEmitterItem {
                target_index,
                ..authored.emitters[0].particle_items[0]
            };
            authored.emitters[0].particle_items.push(item);
        }
        let mut private = authored.clone();
        let materials = scene::ParticleMaterials::prepare(&mut private);
        let sample = |f| {
            let mut playback = VfxPlayback::new(ParticleEnvironment::default().runtime(f));
            playback.advance(0.).unwrap();
            let mut m = vec![];
            playback.sample(&mut vec![], &mut m);
            m
        };
        let original = sample(&authored);
        let mut actual = sample(&private);
        assert_eq!(actual.len(), 2);
        assert!(actual.iter().all(|m| m.model_index == 0));
        let mut flags = vec![];
        for m in &mut actual {
            let flag = materials.mesh(m);
            flags.push((m.depth_offset_type, flag));
        }
        flags.sort_unstable();
        assert_eq!(flags, [(-13, false), (i32::MAX, true)]);
        assert_eq!(actual, original, "restore all public packet values exactly");
    }
}
