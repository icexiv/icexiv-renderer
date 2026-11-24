//! Render an extracted MDL into a PNG using a native offscreen texture.
use icexiv_renderer::{EquipmentModel, Renderer, wgpu as w};
use std::{error::Error, fs, io::BufWriter, sync::mpsc};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let model = EquipmentModel::from_mdl(&fs::read(args.next().ok_or("expected input.mdl")?)?)?;
    let path = args.next().ok_or("expected output.png")?;
    let instance = w::Instance::new(w::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let format = w::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(device.clone(), queue.clone(), format);
    let mut gpu_model = renderer.upload(&model);
    // Optional flat sample directory: one unambiguous basename per resource.
    if let Some(directory) = args.next() {
        let root = std::path::PathBuf::from(directory);
        for (slot, name) in gpu_model.material_names().to_vec().iter().enumerate() {
            let file = root.join(
                std::path::Path::new(name)
                    .file_name()
                    .ok_or("invalid material name")?,
            );
            let bytes = match fs::read(&file) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("neutral slot {slot}: {name}");
                    continue;
                }
                other => other?,
            };
            let texture = icexiv_renderer::base_color_texture(&bytes)?;
            let read_tex = |path: Option<&str>| -> Result<Vec<u8>, Box<dyn Error>> {
                Ok(match path {
                    Some(p) => fs::read(
                        root.join(
                            std::path::Path::new(p)
                                .file_name()
                                .ok_or("invalid texture name")?,
                        ),
                    )?,
                    None => Vec::new(),
                })
            };
            let diffuse = icexiv_renderer::diffuse_overlay(&bytes)?;
            renderer.set_material_with_diffuse(
                &mut gpu_model,
                slot,
                &bytes,
                &read_tex(texture)?,
                &read_tex(diffuse)?,
            )?;
            eprintln!("slot {slot}: {name} ← {}", texture.unwrap_or("inline"));
        }
    }
    const S: u32 = 512; // 4*S is aligned for texture readback.
    let size = w::Extent3d {
        width: S,
        height: S,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&w::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: w::TextureDimension::D2,
        format,
        usage: w::TextureUsages::RENDER_ATTACHMENT | w::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    renderer.draw(
        &target.create_view(&Default::default()),
        [S, S],
        &gpu_model,
        0.35,
        0.1,
    )?;
    let buffer = device.create_buffer(&w::BufferDescriptor {
        label: None,
        size: u64::from(4 * S * S),
        mapped_at_creation: false,
        usage: w::BufferUsages::COPY_DST | w::BufferUsages::MAP_READ,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        w::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: w::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * S),
                rows_per_image: Some(S),
            },
        },
        size,
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, recv) = mpsc::channel();
    buffer.slice(..).map_async(w::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device.poll(w::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    recv.recv()??;
    let pixels = buffer.slice(..).get_mapped_range();
    let covered = pixels.chunks_exact(4).filter(|p| p[3] != 0).count();
    if covered == 0 {
        return Err("model produced no visible fragments".into());
    }
    let mut png = png::Encoder::new(BufWriter::new(fs::File::create(path)?), S, S);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&pixels)?;
    println!(
        "{} triangles, {covered} visible pixels",
        model.triangle_count()
    );
    Ok(())
}
