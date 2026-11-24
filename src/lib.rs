//! FFXIV static equipment preview. Input is a logical MDL extracted from SqPack.
mod material;
mod model;
mod paths;
pub use material::{MaterialError, base_color_texture, color_texture_path, diffuse_overlay};
pub use paths::material_paths;

/// Exercise the same color decode as `setMaterial` without a GPU (asset preparation).
pub fn validate_material(
    material: &[u8],
    texture: &[u8],
    max_side: u32,
) -> Result<(), MaterialError> {
    material::Pixels::decode(material, texture, max_side).map(|_| ())
}

pub fn validate_material_with_diffuse(
    material: &[u8],
    texture: &[u8],
    diffuse: &[u8],
    max_side: u32,
) -> Result<(), MaterialError> {
    material::Pixels::decode_with_diffuse(material, texture, diffuse, max_side).map(|_| ())
}

/// Material directory selector from an IMC subset and part mask bit.
pub fn material_variant(imc: &[u8], subset: u16, part: u8) -> Result<u8, icexiv_data::imc::Error> {
    Ok(icexiv_data::imc::Imc::parse(imc)?
        .entry(subset, part)?
        .material_id)
}
mod render;
pub use model::{EquipmentModel, ModelError};
pub use render::{GpuModel, Renderer};
pub use wgpu;

#[cfg(target_arch = "wasm32")]
mod web;
