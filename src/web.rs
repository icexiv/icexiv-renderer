use crate::{EquipmentModel, GpuModel, Renderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu as w;

fn error(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// A standalone pose sampler; no DOM, canvas or GPU device is required.
#[wasm_bindgen]
pub struct SkeletonClip(crate::PapPose);

/// Names in PAP record order, including additive records. The index can be
/// passed directly to SkeletonClip.load, loadLayered or setAnimatedModel.
#[wasm_bindgen(js_name = animationRecords)]
pub fn animation_records(bytes: &[u8]) -> Result<Vec<String>, JsError> {
    let pap = crate::animation::parse_pap(bytes).map_err(error)?;
    Ok(pap
        .records()
        .map(|r| String::from_utf8_lossy(r.name).into_owned())
        .collect())
}

#[wasm_bindgen]
impl SkeletonClip {
    #[wasm_bindgen(js_name = loadNamed)]
    pub fn load_named(sklb: &[u8], pap: Vec<u8>, name: &str) -> Result<SkeletonClip, JsError> {
        crate::PapPose::named(sklb, pap, name.as_bytes())
            .map(Self)
            .map_err(error)
    }
    pub fn load(sklb: &[u8], pap: Vec<u8>, record: usize) -> Result<SkeletonClip, JsError> {
        crate::PapPose::new(sklb, pap, record)
            .map(Self)
            .map_err(error)
    }

    /// Normal base plus one additive record, both driven by boneWorld's seconds.
    #[wasm_bindgen(js_name = loadLayered)]
    pub fn load_layered(
        sklb: &[u8],
        base: Vec<u8>,
        record: usize,
        overlay: Vec<u8>,
        overlay_record: usize,
        weight: f32,
    ) -> Result<SkeletonClip, JsError> {
        crate::PapPose::with_additive(sklb, base, record, overlay, overlay_record, weight)
            .map(Self)
            .map_err(error)
    }

    #[wasm_bindgen(js_name = durationSeconds)]
    pub fn duration_seconds(&self) -> f64 {
        self.0.duration_seconds()
    }
    /// Evaluate once, mix as needed, then read multiple bones without seeking.
    pub fn sample(&mut self, seconds: f64) -> Result<(), JsError> {
        if !(0. ..=self.0.duration_seconds()).contains(&seconds) {
            return Err(error("time outside clip duration"));
        }
        self.0.sample(seconds).map_err(error)
    }
    #[wasm_bindgen(js_name=blendCurrent)]
    pub fn blend_current(
        &mut self,
        other: &SkeletonClip,
        weight: f32,
        subtree: Option<String>,
    ) -> Result<(), JsError> {
        self.0
            .blend_current(&other.0, weight, subtree.as_deref().map(str::as_bytes))
            .map_err(error)
    }
    #[wasm_bindgen(js_name=currentBoneWorld)]
    pub fn current_bone_world(&self, name: &str) -> Result<Vec<f32>, JsError> {
        self.0
            .bone_world(name.as_bytes())
            .map(|m| m.to_cols_array().to_vec())
            .ok_or_else(|| error(format!("unknown bone: {name}")))
    }
    #[wasm_bindgen(js_name=currentLocalPose)]
    pub fn current_local_pose(&self, names: Vec<String>) -> Result<Vec<f32>, JsError> {
        self.0
            .local_pose(names.iter().map(String::as_bytes))
            .map_err(error)
    }
    #[wasm_bindgen(js_name = restBoneWorld)]
    pub fn rest_bone_world(&self, name: &str) -> Result<Vec<f32>, JsError> {
        self.0
            .rest_bone_world(name.as_bytes())
            .map(|m| m.to_cols_array().to_vec())
            .ok_or_else(|| error(format!("unknown bone: {name}")))
    }
    #[wasm_bindgen(js_name = localPose)]
    pub fn local_pose(&mut self, names: Vec<String>, seconds: f64) -> Result<Vec<f32>, JsError> {
        self.sample(seconds)?;
        self.current_local_pose(names)
    }

    #[wasm_bindgen(js_name = boneWorld)]
    pub fn bone_world(&mut self, name: &str, seconds: f64) -> Result<Vec<f32>, JsError> {
        self.sample(seconds)?;
        self.current_bone_world(name)
    }
}

#[wasm_bindgen]
pub struct EquipmentViewer {
    canvas: HtmlCanvasElement,
    surface: w::Surface<'static>,
    config: w::SurfaceConfiguration,
    device: w::Device,
    renderer: Renderer,
    model: Option<GpuModel>,
}

#[wasm_bindgen(js_name = colorTexturePath)]
pub fn color_texture_path(material: &[u8]) -> Result<String, JsError> {
    crate::color_texture_path(material)
        .map(str::to_owned)
        .map_err(error)
}

#[wasm_bindgen(js_name = materialVariant)]
pub fn material_variant(imc: &[u8], subset: u16, part: u8) -> Result<u8, JsError> {
    crate::material_variant(imc, subset, part).map_err(error)
}

#[wasm_bindgen(js_name = baseColorTexture)]
pub fn base_color_texture(material: &[u8]) -> Result<Option<String>, JsError> {
    crate::base_color_texture(material)
        .map(|p| p.map(str::to_owned))
        .map_err(error)
}

#[wasm_bindgen(js_name = materialPaths)]
pub fn material_paths(model: &str, reference: &str, variant: Option<u16>) -> Vec<String> {
    crate::material_paths(model, reference, variant)
}

#[wasm_bindgen(js_name = diffuseOverlay)]
pub fn diffuse_overlay(material: &[u8]) -> Result<Option<String>, JsError> {
    crate::diffuse_overlay(material)
        .map(|p| p.map(str::to_owned))
        .map_err(error)
}

#[wasm_bindgen]
impl EquipmentViewer {
    pub async fn create(canvas: HtmlCanvasElement) -> Result<EquipmentViewer, JsError> {
        let instance = w::Instance::new(w::InstanceDescriptor {
            backends: w::Backends::BROWSER_WEBGPU,
            ..w::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(w::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(error)?;
        let adapter = instance
            .request_adapter(&w::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(error)?;
        let (device, queue) = adapter
            .request_device(&Default::default())
            .await
            .map_err(error)?;
        let [width, height] = [canvas.width().max(1), canvas.height().max(1)];
        if width.max(height) > device.limits().max_texture_dimension_2d {
            return Err(error("canvas exceeds WebGPU device limit"));
        }
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| error("no WebGPU surface configuration"))?;
        let view_format = config.format.add_srgb_suffix();
        if view_format != config.format {
            config.view_formats.push(view_format);
        }
        if surface
            .get_capabilities(&adapter)
            .alpha_modes
            .contains(&w::CompositeAlphaMode::PreMultiplied)
        {
            config.alpha_mode = w::CompositeAlphaMode::PreMultiplied;
        }
        let msaa = [view_format, w::TextureFormat::Depth24Plus]
            .iter()
            .all(|&f| {
                adapter
                    .get_texture_format_features(f)
                    .flags
                    .sample_count_supported(4)
            });
        let renderer =
            Renderer::with_samples(device.clone(), queue, view_format, if msaa { 4 } else { 1 });
        surface.configure(&device, &config);
        Ok(Self {
            canvas,
            surface,
            config,
            device,
            renderer,
            model: None,
        })
    }

    #[wasm_bindgen(js_name = setModel)]
    pub fn set_model(&mut self, mdl: &[u8]) -> Result<(), JsError> {
        let model = EquipmentModel::from_mdl(mdl).map_err(error)?;
        self.model = Some(self.renderer.upload(&model));
        Ok(())
    }

    /// Replace the model and initialize it at the selected PAP record's first frame.
    #[wasm_bindgen(js_name = setAnimatedModel)]
    pub fn set_animated_model(
        &mut self,
        mdl: &[u8],
        sklb: &[u8],
        pap: Vec<u8>,
        record: usize,
    ) -> Result<f64, JsError> {
        let pose = crate::PapPose::new(sklb, pap, record).map_err(error)?;
        let duration = pose.duration_seconds();
        let animation = crate::AnimatedEquipment::new(mdl, pose).map_err(error)?;
        self.model = Some(self.renderer.upload_animated(animation));
        Ok(duration)
    }

    #[wasm_bindgen(js_name = seekAnimation)]
    pub fn seek_animation(&mut self, seconds: f64) -> Result<(), JsError> {
        self.renderer
            .seek_animation(
                self.model
                    .as_mut()
                    .ok_or_else(|| error("no model loaded"))?,
                seconds,
            )
            .map_err(error)
    }

    /// RGB components are sRGB bytes, like a CSS hex color.
    #[wasm_bindgen(js_name=setBackground)]
    pub fn set_background(&mut self, r: u8, g: u8, b: u8) {
        let linear = |v: u8| {
            let x = f64::from(v) / 255.;
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        };
        self.renderer.set_background(w::Color {
            r: linear(r),
            g: linear(g),
            b: linear(b),
            a: 1.,
        });
    }

    pub fn draw(&mut self, yaw: f32, pitch: f32) -> Result<(), JsError> {
        self.draw_zoom(yaw, pitch, 1.)
    }
    #[wasm_bindgen(js_name=drawZoom)]
    pub fn draw_zoom(&mut self, yaw: f32, pitch: f32, zoom: f32) -> Result<(), JsError> {
        let model = self
            .model
            .as_ref()
            .ok_or_else(|| error("call setModel first"))?;
        let size = [self.canvas.width(), self.canvas.height()];
        if size.contains(&0) {
            return Ok(());
        }
        if size
            .iter()
            .any(|&s| s > self.device.limits().max_texture_dimension_2d)
        {
            return Err(error("canvas exceeds WebGPU device limit"));
        }
        if size != [self.config.width, self.config.height] {
            [self.config.width, self.config.height] = size;
            self.surface.configure(&self.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            w::CurrentSurfaceTexture::Success(t) | w::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => return Err(error(format!("cannot acquire WebGPU frame: {other:?}"))),
        };
        self.renderer
            .draw_zoom(
                &frame.texture.create_view(&w::TextureViewDescriptor {
                    format: Some(self.config.format.add_srgb_suffix()),
                    ..Default::default()
                }),
                size,
                model,
                yaw,
                pitch,
                zoom,
            )
            .map_err(error)?;
        frame.present();
        Ok(())
    }

    #[wasm_bindgen(js_name = materialNames)]
    pub fn material_names(&self) -> Result<Vec<String>, JsError> {
        Ok(self
            .model
            .as_ref()
            .ok_or_else(|| error("call setModel first"))?
            .material_names()
            .to_vec())
    }

    #[wasm_bindgen(js_name = setMaterial)]
    pub fn set_material(
        &mut self,
        slot: usize,
        material: &[u8],
        texture: &[u8],
    ) -> Result<(), JsError> {
        self.renderer
            .set_material(
                self.model
                    .as_mut()
                    .ok_or_else(|| error("call setModel first"))?,
                slot,
                material,
                texture,
            )
            .map_err(error)
    }

    #[wasm_bindgen(js_name = setMaterialWithDiffuse)]
    pub fn set_material_with_diffuse(
        &mut self,
        slot: usize,
        material: &[u8],
        texture: &[u8],
        diffuse: &[u8],
    ) -> Result<(), JsError> {
        self.renderer
            .set_material_with_diffuse(
                self.model
                    .as_mut()
                    .ok_or_else(|| error("call setModel first"))?,
                slot,
                material,
                texture,
                diffuse,
            )
            .map_err(error)
    }
}
