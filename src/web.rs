use crate::{EquipmentModel, GpuModel, Renderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu as w;

fn error(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
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
}
