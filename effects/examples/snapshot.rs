//! Offline acceptance: compare actual mounted frames to the same render with effects disabled.
//! Args: export JSON, CAS directory, PNG output directory, optional yaw and pitch (radians).
//! Append extracted action root, events JSON and preview JSON for combined acceptance.
use anyhow::{Result, ensure};
use icexiv_effects::{EffectAssets, EffectPlayer};
use std::{collections::HashMap, fs, path::PathBuf, sync::mpsc};
use wgpu as w;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let desc: serde_json::Value = serde_json::from_slice(&fs::read(args.next().unwrap())?)?;
    let root = args.next().unwrap();
    let out = args.next().unwrap();
    let angle = |arg: Option<PathBuf>, default: f32| -> Result<f32> {
        Ok(match arg {
            Some(value) => value.to_string_lossy().parse()?,
            None => default,
        })
    };
    let yaw = angle(args.next(), 0.65)?;
    let pitch = angle(args.next(), 0.35)?;
    fs::create_dir_all(&out)?;
    let mut files = HashMap::new();
    for (path, h) in desc["files"].as_object().unwrap() {
        let h = h.as_str().unwrap();
        files.insert(path.clone(), fs::read(root.join(&h[..2]).join(&h[2..]))?);
    }
    let main = desc["main"].as_str().unwrap().parse()?;
    let mut source = assets::resource::MemoryResource::new(files);
    let instance = w::Instance::new(w::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&w::DeviceDescriptor {
        required_limits: scene::ModelRenderContext::required_limits(adapter.limits()),
        ..Default::default()
    }))?;
    let format = w::TextureFormat::Rgba8UnormSrgb;
    const S: u32 = 256;
    let size = w::Extent3d {
        width: S,
        height: S,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&w::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: w::TextureDimension::D2,
        format,
        usage: w::TextureUsages::RENDER_ATTACHMENT | w::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    if let Some(extracted) = args.next() {
        let events = fs::read(
            args.next()
                .ok_or_else(|| anyhow::anyhow!("expected action events"))?,
        )?;
        let preview = fs::read(
            args.next()
                .ok_or_else(|| anyhow::anyhow!("expected action preview"))?,
        )?;
        let mut overlap = Vec::new();
        let mut after = Vec::new();
        for mode in ["resident", "action", "combined"] {
            let a = EffectAssets::load(&mut source, main)?;
            let mut player = EffectPlayer::new(device.clone(), queue.clone(), format, a);
            player.set_resident_visible(mode != "action");
            for tick in 0..=180 {
                if tick == 60 && mode != "resident" {
                    let (action, draw) = icexiv_effects::timeline::ActionPlayer::load(
                        serde_json::from_slice(&events)?,
                        serde_json::from_slice(&preview)?,
                        |p| {
                            ensure!(
                                std::path::Path::new(p)
                                    .components()
                                    .all(|v| matches!(v, std::path::Component::Normal(_))),
                                "invalid action path"
                            );
                            Ok(fs::read(extracted.join(p))?)
                        },
                    )?;
                    player.play_action(action, draw);
                }
                player.frame(
                    &view,
                    [S, S],
                    yaw,
                    pitch,
                    3.2,
                    if tick == 0 { 0. } else { 1. / 60. },
                    true,
                )?;
                if matches!(tick, 84 | 180) {
                    let pixels = read_pixels(&device, &queue, &texture, size)?;
                    save(&out.join(format!("{mode}-{tick}.png")), S, &pixels)?;
                    if tick == 84 {
                        overlap.push(pixels)
                    } else {
                        after.push(pixels)
                    }
                }
            }
        }
        let changed = |a: &[u8], b: &[u8]| {
            a.chunks_exact(4)
                .zip(b.chunks_exact(4))
                .filter(|(a, b)| a[..3] != b[..3])
                .count()
        };
        let action_pixels = changed(&overlap[2], &overlap[0]);
        let resident_pixels = changed(&overlap[2], &overlap[1]);
        println!(
            "overlap: {action_pixels} pixels differ from resident only; {resident_pixels} from action only"
        );
        ensure!(
            action_pixels != 0 && resident_pixels != 0,
            "both sources must contribute"
        );
        ensure!(
            after[2] == after[0],
            "resident phase changed after action retirement"
        );
        return Ok(());
    }
    let a = EffectAssets::load(&mut source, main)?;
    let reference = EffectAssets::load(&mut source, main)?;
    let mut player = EffectPlayer::new(device.clone(), queue.clone(), format, a);
    // Separate contexts keep screen-copy history and particle retirement out
    // of the no-effect reference, with identical camera and MTRL scroll time.
    let mut reference = EffectPlayer::new(device.clone(), queue.clone(), format, reference);
    let mut contributions = Vec::new();
    for tick in -1..=240 {
        let delta = if tick <= 0 { 0. } else { 1. / 60. };
        reference.frame(&view, [S, S], yaw, pitch, 3.2, delta, false)?;
        let capture = matches!(tick, -1 | 48 | 240);
        let baseline = if capture {
            let pixels = read_pixels(&device, &queue, &texture, size)?;
            let name = if tick == -1 {
                "frame--1.png".to_owned()
            } else {
                format!("baseline-{tick}.png")
            };
            save(&out.join(name), S, &pixels)?;
            Some(pixels)
        } else {
            None
        };
        player.frame(&view, [S, S], yaw, pitch, 3.2, delta, tick >= 0)?;
        if tick < 0 {
            continue;
        }
        let Some(baseline) = baseline else {
            continue;
        };
        for warning in player.diagnostics() {
            eprintln!("tick {tick}: {warning}");
        }
        let pixels = read_pixels(&device, &queue, &texture, size)?;
        save(&out.join(format!("frame-{tick}.png")), S, &pixels)?;
        let difference: Vec<[i16; 3]> = pixels
            .chunks_exact(4)
            .zip(baseline.chunks_exact(4))
            .map(|(a, b)| std::array::from_fn(|i| i16::from(a[i]) - i16::from(b[i])))
            .collect();
        let changed = difference.iter().filter(|rgb| **rgb != [0; 3]).count();
        println!("tick {tick}: {changed} pixels differ from same-time no-effect baseline");
        ensure!(changed > 0, "mounted effect not visible");
        contributions.push(difference);
    }
    ensure!(
        contributions[0] != contributions[1],
        "effect contribution did not change"
    );
    Ok(())
}

fn read_pixels(
    device: &w::Device,
    queue: &w::Queue,
    texture: &w::Texture,
    size: w::Extent3d,
) -> Result<Vec<u8>> {
    let buffer = device.create_buffer(&w::BufferDescriptor {
        label: None,
        size: u64::from(4 * size.width * size.height),
        usage: w::BufferUsages::COPY_DST | w::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        w::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: w::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    let submission = queue.submit([enc.finish()]);
    let (tx, rx) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(w::MapMode::Read, move |v| tx.send(v).unwrap());
    device.poll(w::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    rx.recv()??;
    Ok(buffer.slice(..).get_mapped_range().to_vec())
}
fn save(path: &std::path::Path, side: u32, pixels: &[u8]) -> Result<()> {
    let mut png = png::Encoder::new(fs::File::create(path)?, side, side);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(pixels)?;
    Ok(())
}
