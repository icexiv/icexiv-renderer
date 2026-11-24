use crate::EquipmentModel;
use glam::{Mat4, Vec3};
use wgpu::{self as w, util::DeviceExt};

pub struct GpuModel {
    vertices: w::Buffer,
    indices: w::Buffer,
    parts: Vec<(usize, std::ops::Range<u32>)>,
    names: Vec<String>,
    materials: Vec<w::BindGroup>,
    cull: Vec<bool>,
}
impl GpuModel {
    pub fn material_names(&self) -> &[String] {
        &self.names
    }
}

/// Single-sampled renderer. A native host supplies its device, queue and color view.
pub struct Renderer {
    device: w::Device,
    queue: w::Queue,
    pipeline: w::RenderPipeline,
    culled: w::RenderPipeline,
    samples: u32,
    format: w::TextureFormat,
    clear: w::Color,
    color: Option<w::TextureView>,
    camera: w::Buffer,
    group: w::BindGroup,
    depth: Option<([u32; 2], w::TextureView)>,
}

impl Renderer {
    pub fn new(device: w::Device, queue: w::Queue, format: w::TextureFormat) -> Self {
        Self::with_samples(device, queue, format, 1)
    }
    pub(crate) fn with_samples(
        device: w::Device,
        queue: w::Queue,
        format: w::TextureFormat,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(w::ShaderModuleDescriptor {
            label: Some("equipment preview"),
            source: w::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let layout = |entries: &[w::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&w::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        };
        let camera_layout = layout(&[w::BindGroupLayoutEntry {
            binding: 0,
            visibility: w::ShaderStages::VERTEX_FRAGMENT,
            ty: w::BindingType::Buffer {
                ty: w::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);
        let material_layout = layout(&[
            w::BindGroupLayoutEntry {
                binding: 0,
                visibility: w::ShaderStages::FRAGMENT,
                ty: w::BindingType::Texture {
                    sample_type: w::TextureSampleType::Float { filterable: true },
                    view_dimension: w::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            w::BindGroupLayoutEntry {
                binding: 1,
                visibility: w::ShaderStages::FRAGMENT,
                ty: w::BindingType::Sampler(w::SamplerBindingType::Filtering),
                count: None,
            },
        ]);
        let pipeline_layout = device.create_pipeline_layout(&w::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&camera_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let make_pipeline = |cull_mode| {
            device.create_render_pipeline(&w::RenderPipelineDescriptor {
            label: Some("equipment preview"),
            layout: Some(&pipeline_layout),
            vertex: w::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[w::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: w::VertexStepMode::Vertex,
                    attributes: &w::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                }],
            },
            fragment: Some(w::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: w::PrimitiveState{cull_mode,..Default::default()},
            depth_stencil: Some(w::DepthStencilState {
                format: w::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(w::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: w::MultisampleState{count:samples,..Default::default()},
            multiview_mask: None,
            cache: None,
        })
        };
        let pipeline = make_pipeline(None);
        let culled = make_pipeline(Some(w::Face::Back));
        let camera = device.create_buffer(&w::BufferDescriptor {
            label: Some("preview camera"),
            size: 80,
            mapped_at_creation: false,
            usage: w::BufferUsages::UNIFORM | w::BufferUsages::COPY_DST,
        });
        let group = device.create_bind_group(&w::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[w::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            }],
        });
        Self {
            device,
            queue,
            pipeline,
            culled,
            samples,
            format,
            clear: w::Color::TRANSPARENT,
            color: None,
            camera,
            group,
            depth: None,
        }
    }

    pub fn upload(&self, model: &EquipmentModel) -> GpuModel {
        let buffer = |label, bytes, usage| {
            self.device
                .create_buffer_init(&w::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage,
                })
        };
        let fallback = self.material_group(crate::material::Pixels {
            size: [1, 1],
            rgba: vec![184, 194, 204, 255],
            srgb: false,
        });
        GpuModel {
            vertices: buffer(
                "equipment vertices",
                bytemuck::cast_slice(&model.vertices),
                w::BufferUsages::VERTEX,
            ),
            indices: buffer(
                "indices",
                bytemuck::cast_slice(&model.indices),
                w::BufferUsages::INDEX,
            ),
            parts: model.parts.clone(),
            names: model.names.clone(),
            materials: vec![fallback; model.names.len()],
            cull: vec![false; model.names.len()],
        }
    }

    fn material_group(&self, mut pixels: crate::material::Pixels) -> w::BindGroup {
        let mip_count = pixels.append_mips();
        let texture = self.device.create_texture_with_data(
            &self.queue,
            &w::TextureDescriptor {
                label: Some("equipment material color"),
                size: w::Extent3d {
                    width: pixels.size[0],
                    height: pixels.size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: mip_count,
                sample_count: 1,
                dimension: w::TextureDimension::D2,
                format: if pixels.srgb {
                    w::TextureFormat::Rgba8UnormSrgb
                } else {
                    w::TextureFormat::Rgba8Unorm
                },
                usage: w::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            w::util::TextureDataOrder::LayerMajor,
            &pixels.rgba,
        );
        let sampler = self.device.create_sampler(&w::SamplerDescriptor {
            address_mode_u: w::AddressMode::Repeat,
            address_mode_v: w::AddressMode::Repeat,
            mag_filter: w::FilterMode::Linear,
            min_filter: w::FilterMode::Linear,
            mipmap_filter: w::MipmapFilterMode::Linear,
            ..Default::default()
        });
        self.device.create_bind_group(&w::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(1),
            entries: &[
                w::BindGroupEntry {
                    binding: 0,
                    resource: w::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                w::BindGroupEntry {
                    binding: 1,
                    resource: w::BindingResource::Sampler(&sampler),
                },
            ],
        })
    }

    pub fn set_material(
        &self,
        model: &mut GpuModel,
        slot: usize,
        material: &[u8],
        texture: &[u8],
    ) -> Result<(), crate::MaterialError> {
        let binding = model
            .materials
            .get_mut(slot)
            .ok_or(crate::MaterialError::Unsupported(
                "material slot outside model",
            ))?;
        let hide_backfaces = icexiv_data::mtrl::Mtrl::parse(material)?.shader_flags() & 1 != 0;
        *binding = self.material_group(crate::material::Pixels::decode(
            material,
            texture,
            self.device.limits().max_texture_dimension_2d,
        )?);
        model.cull[slot] = hide_backfaces;
        Ok(())
    }

    /// Clear and draw into a view of the format passed to `new`. Size is in pixels;
    /// orbit angles are radians. Submit before the host presents the surface.
    pub fn draw(
        &mut self,
        target: &w::TextureView,
        size: [u32; 2],
        model: &GpuModel,
        yaw: f32,
        pitch: f32,
    ) -> Result<(), &'static str> {
        self.draw_zoom(target, size, model, yaw, pitch, 1.)
    }
    pub fn draw_zoom(
        &mut self,
        target: &w::TextureView,
        size: [u32; 2],
        model: &GpuModel,
        yaw: f32,
        pitch: f32,
        zoom: f32,
    ) -> Result<(), &'static str> {
        let limit = self.device.limits().max_texture_dimension_2d;
        if size.iter().any(|&s| s == 0 || s > limit)
            || !yaw.is_finite()
            || !pitch.is_finite()
            || !zoom.is_finite()
        {
            return Err("invalid viewport or camera");
        }
        if self.depth.as_ref().map(|d| d.0) != Some(size) {
            let attachment = |format| {
                self.device
                    .create_texture(&w::TextureDescriptor {
                        label: Some("equipment render attachment"),
                        size: w::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: self.samples,
                        dimension: w::TextureDimension::D2,
                        format,
                        usage: w::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            self.color = (self.samples > 1).then(|| attachment(self.format));
            self.depth = Some((size, attachment(w::TextureFormat::Depth24Plus)));
        }
        // Orbit basis and camera-relative studio light extracted from Companion's
        // renderer/model/material/uniforms.rs (camera_uniform_and_vfx_view).
        let aspect = size[0] as f32 / size[1] as f32;
        let pitch = pitch.clamp(-1.35, 1.35);
        let dir = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = Vec3::Y.cross(dir).normalize();
        let up = dir.cross(right).normalize();
        let distance = (2.8 / zoom.clamp(0.65, 1.8)).max(1.45) / aspect.min(1.0);
        let view = Mat4::look_at_rh(dir * distance, Vec3::ZERO, up);
        let projection = Mat4::perspective_rh(
            45_f32.to_radians(),
            aspect,
            distance - 1.05,
            distance + 1.05,
        );
        let light = (-0.45 * right + 0.65 * up + 0.65 * dir).normalize();
        let mut uniform = [0f32; 20];
        uniform[..16].copy_from_slice(&(projection * view).to_cols_array());
        uniform[16..19].copy_from_slice(&light.to_array());
        self.queue
            .write_buffer(&self.camera, 0, bytemuck::cast_slice(&uniform));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&w::RenderPassDescriptor {
                label: Some("equipment preview"),
                color_attachments: &[Some(w::RenderPassColorAttachment {
                    view: self.color.as_ref().unwrap_or(target),
                    depth_slice: None,
                    resolve_target: self.color.as_ref().map(|_| target),
                    ops: w::Operations {
                        load: w::LoadOp::Clear(self.clear),
                        store: if self.color.is_some() {
                            w::StoreOp::Discard
                        } else {
                            w::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(w::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().unwrap().1,
                    stencil_ops: None,
                    depth_ops: Some(w::Operations {
                        load: w::LoadOp::Clear(1.),
                        store: w::StoreOp::Discard,
                    }),
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, model.vertices.slice(..));
            pass.set_index_buffer(model.indices.slice(..), w::IndexFormat::Uint32);
            for (slot, range) in &model.parts {
                pass.set_pipeline(if model.cull[*slot] {
                    &self.culled
                } else {
                    &self.pipeline
                });
                pass.set_bind_group(1, &model.materials[*slot], &[]);
                pass.draw_indexed(range.clone(), 0, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Linear RGBA clear color. Native and low-level viewers default to transparent.
    pub fn set_background(&mut self, color: w::Color) {
        self.clear = color;
    }
}
