//! Controlled C012s on Stone's real shared hand effect, through the compositor.
use anyhow::{Result, ensure};
use icexiv_effects::{EffectAssets, EffectPlayer, timeline::ActionPlayer};
use physis::resource::{Resource, SqPackResource};
use serde_json::json;
use wgpu as w;

#[test]
#[ignore = "requires XIV_GAME_DIR and native wgpu"]
fn c012_shared_asset_pixels() -> Result<()> {
    let mut game = SqPackResource::from_existing(&std::env::var("XIV_GAME_DIR")?);
    let owner = EffectAssets::load_owner(&mut game, 17180398476)?;
    let instance = w::Instance::new(w::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (dev, queue) = pollster::block_on(adapter.request_device(&w::DeviceDescriptor {
        required_limits: scene::ModelRenderContext::required_limits(adapter.limits()),
        ..Default::default()
    }))?;
    let format = w::TextureFormat::Rgba8UnormSrgb;
    let size = w::Extent3d {
        width: 128,
        height: 128,
        depth_or_array_layers: 1,
    };
    let target = dev.create_texture(&w::TextureDescriptor {
        label: None,
        size,
        format,
        mip_level_count: 1,
        sample_count: 1,
        dimension: w::TextureDimension::D2,
        view_formats: &[],
        usage: w::TextureUsages::COPY_SRC | w::TextureUsages::RENDER_ATTACHMENT,
    });
    let view = target.create_view(&Default::default());
    let mut player = EffectPlayer::new(dev.clone(), queue.clone(), format, owner);
    let mut render = |colors: [[f32; 4]; 2], revised: [f32; 3]| -> Result<Vec<u8>> {
        player.reset();
        let events = colors.iter().enumerate().map(|(i, color)| json!({
            "avfx":"vfx/common/eff/cmcn_shoot1c.avfx", "chain":[{"tmb":"test","actor":0,"track":0,"command":1,
            "actor_start":0,"track_start":0,"start":0,"duration":60,"ability_delay":0}],
            "targets":[{"origin":1,"kind":i+1,"id":-1},{"origin":2,"kind":0,"id":-1}],
            "color":color,"visibility":0,"scale":[1,1,1],"rotation_radians":[0,0,0],"position":[0,0,0]
        })).collect::<Vec<_>>();
        let mut reads = 0;
        let (action, draw) = ActionPlayer::load(
            serde_json::from_value(json!(events))?,
            serde_json::from_str(include_str!("../examples/stone-preview.json"))?,
            |path| {
                let mut bytes = game.read(path)?;
                if path.ends_with(".avfx") {
                    reads += 1;
                    // Replace only root RvR/G/B payloads in the controlled copy.
                    let mut pos = 8;
                    let mut found = 0;
                    while pos + 8 <= bytes.len() {
                        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into()?) as usize;
                        for (axis, key) in [b"RvR\0", b"GvR\0", b"BvR\0"].iter().enumerate() {
                            if &bytes[pos..pos + 4] == *key {
                                ensure!(len == 4);
                                bytes[pos + 8..pos + 12]
                                    .copy_from_slice(&revised[axis].to_le_bytes());
                                found += 1;
                            }
                        }
                        pos += 8 + len.next_multiple_of(4);
                    }
                    ensure!(found == 3);
                }
                Ok(bytes)
            },
        )?;
        assert_eq!(reads, 1);
        player.play_action(action, draw);
        for tick in 0..=24 {
            player.frame(
                &view,
                [128; 2],
                0.65,
                0.35,
                3.2,
                if tick == 0 { 0. } else { 1. / 60. },
                true,
            )?;
        }
        assert_eq!(
            player
                .action_player_mut()
                .unwrap()
                .active_occurrences()
                .count(),
            2
        );
        let buffer = dev.create_buffer(&w::BufferDescriptor {
            label: None,
            size: 128 * 128 * 4,
            usage: w::BufferUsages::COPY_DST | w::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut cmd = dev.create_command_encoder(&Default::default());
        cmd.copy_texture_to_buffer(
            target.as_image_copy(),
            w::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: w::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(128),
                },
            },
            size,
        );
        let index = queue.submit([cmd.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(w::MapMode::Read, move |v| tx.send(v).unwrap());
        dev.poll(w::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })?;
        rx.recv()??;
        Ok(buffer.slice(..).get_mapped_range().to_vec())
    };
    let white = [1.; 4];
    let clear = [1., 1., 1., 0.];
    let tint = [2., 0.25, 0., 1.];
    let unit = [1.; 3];
    let blank = render([clear; 2], unit)?;
    let both = render([white; 2], unit)?;
    let left = render([white, clear], unit)?;
    let right = render([clear, white], unit)?;
    let different = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
    };
    for (a, b) in [
        (&blank, &left),
        (&blank, &right),
        (&both, &left),
        (&both, &right),
        (&left, &right),
    ] {
        assert!(different(a, b) > 16);
    }
    let tinted = render([tint; 2], unit)?;
    assert!(different(&both, &tinted) > 16);
    assert_eq!(different(&tinted, &render([white; 2], [2., 0.25, 0.])?), 0);
    assert_eq!(
        different(
            &render([[0., 0., 0., 1.]; 2], unit)?,
            &render([white; 2], [0.; 3])?
        ),
        0
    );
    let half = render([[1., 1., 1., 0.5]; 2], unit)?;
    assert!(different(&both, &half) > 16 && different(&blank, &half) > 16);
    Ok(())
}
