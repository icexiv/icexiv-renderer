use glam::Vec3;
use icexiv_data::mdl::{Mdl, MeshRole};

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error(transparent)]
    Format(#[from] icexiv_data::mdl::Error),
    #[error("invalid equipment geometry: {0}")]
    Geometry(&'static str),
    #[error("model positions coincide; no visible extent")]
    NoVisibleExtent,
}

/// Bind-pose triangle geometry, normalized for a bounds-fitted orbit camera.
pub struct EquipmentModel {
    pub(crate) vertices: Vec<[f32; 8]>,
    pub(crate) indices: Vec<u32>,
    pub(crate) parts: Vec<(usize, std::ops::Range<u32>)>,
    pub(crate) names: Vec<String>,
    pub(crate) meshes: Vec<usize>,
    pub(crate) center: Vec3,
    pub(crate) radius: f32,
}

impl EquipmentModel {
    pub fn from_mdl(bytes: &[u8]) -> Result<Self, ModelError> {
        use ModelError::Geometry;
        let mdl = Mdl::parse(bytes)?;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut parts = Vec::new();
        let mut meshes = Vec::new();
        let names = (0..mdl.material_count())
            .map(|i| {
                std::str::from_utf8(mdl.material(i)?)
                    .map(str::to_owned)
                    .map_err(|_| Geometry("material reference is not UTF-8"))
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = -min;
        let ranges = mdl.lod(0)?.ranges;
        let primary = ranges
            .iter()
            .any(|r| r.count != 0 && matches!(r.role, MeshRole::Normal | MeshRole::Glass));
        let mut slots = vec![None; names.len()];
        let mut active_names = Vec::new();
        for range in ranges {
            if !(matches!(range.role, MeshRole::Normal | MeshRole::Glass)
                || !primary
                    && matches!(
                        range.role,
                        MeshRole::Water
                            | MeshRole::LightShaft
                            | MeshRole::CrestChange
                            | MeshRole::MaterialChange
                            | MeshRole::VerticalFog
                    ))
            {
                continue;
            }
            let start = usize::from(range.start);
            for mesh in start..start + usize::from(range.count) {
                let geometry = mdl.geometry(0, mesh)?;
                let meta = geometry.metadata();
                if meta.index_count == 0 {
                    continue;
                }
                let base = vertices.len();
                if base + usize::from(meta.vertex_count) > 1_000_000
                    || u64::from(meta.index_count) + indices.len() as u64 > 6_000_000
                {
                    return Err(Geometry("preview exceeds geometry budget"));
                }
                if !meta.index_count.is_multiple_of(3) {
                    return Err(Geometry("incomplete triangle"));
                }
                let normal = geometry
                    .declaration()?
                    .find(|e| e.usage == 3 && e.usage_index == 0 && matches!(e.kind, 2 | 3 | 14));
                for vertex in 0..usize::from(meta.vertex_count) {
                    let p = geometry
                        .attribute(vertex, 0, 0)?
                        .ok_or(Geometry("missing position"))?;
                    let p = Vec3::new(p[0], p[1], p[2]);
                    if !p.is_finite() {
                        return Err(Geometry("non-finite position"));
                    }
                    min = min.min(p);
                    max = max.max(p);
                    let n = match normal {
                        Some(e) => {
                            let n = geometry.vertex_element(vertex, e)?;
                            Vec3::new(n[0], n[1], n[2]).normalize_or_zero()
                        }
                        None => Vec3::ZERO,
                    };
                    let uv = geometry.attribute(vertex, 4, 0)?.unwrap_or_default();
                    vertices.push([p.x, p.y, p.z, n.x, n.y, n.z, uv[0], uv[1]]);
                }
                let first = indices.len() as u32;
                for i in geometry.indices()? {
                    if i >= meta.vertex_count {
                        return Err(Geometry("index outside mesh"));
                    }
                    indices.push(base as u32 + u32::from(i));
                }
                let slot = usize::from(meta.material_index);
                if slot >= names.len() {
                    return Err(Geometry("missing mesh material slot"));
                }
                let active = *slots[slot].get_or_insert_with(|| {
                    active_names.push(names[slot].clone());
                    active_names.len() - 1
                });
                parts.push((active, first..indices.len() as u32));
                meshes.push(mesh);
            }
        }
        if indices.is_empty() {
            return Err(Geometry("no visible LOD 0 triangles"));
        }
        let half = max * 0.5 - min * 0.5;
        let radius = half.length();
        if min == max {
            return Err(ModelError::NoVisibleExtent);
        }
        if !radius.is_finite() || radius <= f32::MIN_POSITIVE {
            return Err(Geometry("invalid bounds"));
        }
        let center = min * 0.5 + max * 0.5;
        for p in &mut vertices {
            let normalized = (Vec3::from_slice(p) - center) / radius;
            p[..3].copy_from_slice(&normalized.to_array());
        }
        Ok(Self {
            vertices,
            indices,
            parts,
            names: active_names,
            meshes,
            center,
            radius,
        })
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
    /// Slot order used by the GPU and by `setMaterial` in the web viewer.
    pub fn material_names(&self) -> &[String] {
        &self.names
    }
}
