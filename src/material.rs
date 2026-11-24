use icexiv_data::{
    mtrl::{ColorLayout, Mtrl},
    tex::{Layout, Tex},
};

#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    #[error(transparent)]
    Mtrl(#[from] icexiv_data::mtrl::Error),
    #[error(transparent)]
    Tex(#[from] icexiv_data::tex::Error),
    #[error("{0}")]
    Unsupported(&'static str),
}

enum Source {
    Index,
    Diffuse,
    Solid([u8; 4]),
}

fn source<'a>(m: &Mtrl<'a>, inline: bool) -> Result<(Source, &'a str), MaterialError> {
    let mut diffuse = None;
    let mut index = None;
    for i in 0..m.sampler_count() {
        let s = m.sampler(i)?;
        if s.texture_index == u8::MAX {
            continue;
        }
        let role = match s.id {
            0x565f8fd8
                if m.color_table()?
                    .is_some_and(|t| t.layout() == ColorLayout::Dawntrail) =>
            {
                Source::Index
            }
            0x115306be | 0x1e6fef9c => Source::Diffuse, // g_SamplerDiffuse, g_SamplerColorMap0
            _ => continue,
        };
        let tex = m.texture(usize::from(s.texture_index))?;
        let path = std::str::from_utf8(tex.path)
            .map_err(|_| MaterialError::Unsupported("texture path is not UTF-8"))?;
        match role {
            Source::Index => index = Some((role, path)),
            Source::Diffuse => diffuse = Some((role, path)),
            Source::Solid(_) => unreachable!(),
        }
    }
    // When every row has the same diffuse value an index map cannot
    // affect the base-color pass (including materials with no bound index).
    if inline
        && (index.is_some() || diffuse.is_none())
        && matches!(
            m.shader_package()?,
            b"character.shpk" | b"characterlegacy.shpk" | b"characterreflection.shpk"
        )
        && let Some(table) = m.color_table()?
    {
        let rgb = table.row(0)?.diffuse();
        if rgb.iter().all(|x| x.is_finite())
            && (1..table.row_count()).all(|i| table.row(i).is_ok_and(|r| r.diffuse() == rgb))
        {
            let [r, g, b] = rgb.map(|v| (255. * v.clamp(0., 1.)).round() as u8);
            return Ok((Source::Solid([r, g, b, 255]), ""));
        }
    }
    if inline && index.is_none() && diffuse.is_none() {
        let tint = match m.shader_package()? {
            b"lightshaft.shpk" => Some(0xd27c58b9), // g_Color
            b"water.shpk" => Some(0xd315e728),      // g_WaterDeepColor
            _ => None,
        };
        for i in 0..m.constant_count() {
            if Some(m.constant(i)?.id) != tint {
                continue;
            }
            let mut values = m.constant_floats(i)?;
            if let (Some(r), Some(g), Some(b)) = (values.next(), values.next(), values.next()) {
                let rgb = [r, g, b];
                if rgb.iter().all(|v| v.is_finite()) {
                    let [r, g, b] = rgb.map(|v| (255. * v.clamp(0., 1.)).round() as u8);
                    return Ok((Source::Solid([r, g, b, 255]), ""));
                }
            }
        }
    }
    index.or(diffuse).ok_or(MaterialError::Unsupported(
        "material has no supported color source",
    ))
}

/// Logical sampler TEX for hosts supporting textured materials only.
pub fn color_texture_path(bytes: &[u8]) -> Result<&str, MaterialError> {
    Ok(source(&Mtrl::parse(bytes)?, false)?.1)
}
/// None when the base-color pass needs only MTRL bytes.
pub fn base_color_texture(bytes: &[u8]) -> Result<Option<&str>, MaterialError> {
    let (_, path) = source(&Mtrl::parse(bytes)?, true)?;
    Ok((!path.is_empty()).then_some(path))
}

/// Legacy character materials may modulate a diffuse map with indexed table colors.
/// The primary texture remains unchanged for single-texture SDK consumers.
pub fn diffuse_overlay(bytes: &[u8]) -> Result<Option<&str>, MaterialError> {
    let m = Mtrl::parse(bytes)?;
    if m.shader_package()? == b"characterlegacy.shpk"
        && matches!(source(&m, false), Ok((Source::Index, _)))
    {
        for i in 0..m.sampler_count() {
            let s = m.sampler(i)?;
            if s.id == 0x115306be && s.texture_index != u8::MAX {
                return std::str::from_utf8(m.texture(usize::from(s.texture_index))?.path)
                    .map(Some)
                    .map_err(|_| MaterialError::Unsupported("texture path is not UTF-8"));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_lightshaft_color_does_not_require_a_dummy_texture() {
        let shader = b"lightshaft.shpk\0";
        let mut bytes = vec![0; 16];
        bytes[..4].copy_from_slice(&0x0103_0000u32.to_le_bytes());
        bytes[8..10].copy_from_slice(&(shader.len() as u16).to_le_bytes());
        bytes.extend(shader);
        bytes.extend([12, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend(0xd27c58b9u32.to_le_bytes());
        bytes.extend([0, 0, 12, 0]);
        for f in [0.25f32, 0.5, 1.] {
            bytes.extend(f.to_le_bytes());
        }
        let length = bytes.len() as u16;
        bytes[4..6].copy_from_slice(&length.to_le_bytes());
        assert!(base_color_texture(&bytes).unwrap().is_none());
        let pixels = Pixels::decode(&bytes, &[], 1).unwrap();
        assert_eq!(pixels.rgba, [64, 128, 255, 255]);
        assert!(!pixels.srgb);
        assert!(Pixels::decode(&bytes, &[], 0).is_err());
    }
}

pub(crate) struct Pixels {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
    pub srgb: bool,
}
impl Pixels {
    // Filter the resolved colors, never the encoded color-table row indices.
    pub(crate) fn append_mips(&mut self) -> u32 {
        let [mut w, mut h] = self.size.map(|v| v as usize);
        let mut start = 0;
        let mut count = 1;
        let linear: [f32; 256] = std::array::from_fn(|i| {
            let v = i as f32 / 255.;
            if self.srgb && v > 0.04045 {
                ((v + 0.055) / 1.055).powf(2.4)
            } else if self.srgb {
                v / 12.92
            } else {
                v
            }
        });
        while w > 1 || h > 1 {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let end = self.rgba.len();
            self.rgba.reserve(nw * nh * 4);
            for y in 0..nh {
                for x in 0..nw {
                    let mut sum = [0.; 3];
                    let mut n = 0.;
                    for sy in y * h / nh..(y + 1) * h / nh {
                        for sx in x * w / nw..(x + 1) * w / nw {
                            let i = start + 4 * (sy * w + sx);
                            for (c, s) in sum.iter_mut().enumerate() {
                                *s += linear[self.rgba[i + c] as usize];
                            }
                            n += 1.;
                        }
                    }
                    for s in sum {
                        let v = s / n;
                        let encoded = if self.srgb && v > 0.0031308 {
                            1.055 * v.powf(1. / 2.4) - 0.055
                        } else if self.srgb {
                            12.92 * v
                        } else {
                            v
                        };
                        self.rgba.push((255. * encoded).round() as u8);
                    }
                    self.rgba.push(255);
                }
            }
            (w, h, start) = (nw, nh, end);
            count += 1;
        }
        count
    }
    pub fn decode(material: &[u8], texture: &[u8], max_side: u32) -> Result<Self, MaterialError> {
        Self::decode_with_diffuse(material, texture, &[], max_side)
    }
    pub fn decode_with_diffuse(
        material: &[u8],
        texture: &[u8],
        overlay: &[u8],
        max_side: u32,
    ) -> Result<Self, MaterialError> {
        let m = Mtrl::parse(material)?;
        let (source, _) = source(&m, true)?;
        if !overlay.is_empty() {
            let (size, mut rgba) = decode_tex(overlay, max_side)?;
            let table = m
                .color_table()?
                .ok_or(MaterialError::Unsupported("missing color table"))?;
            let rows = (0..table.row_count())
                .map(|i| table.row(i).map(|r| r.diffuse()))
                .collect::<Result<Vec<_>, _>>()?;
            let index = if matches!(source, Source::Solid(_)) {
                None
            } else {
                Some(decode_tex(texture, max_side)?)
            };
            for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                let tint = if let Some(([w, h], data)) = &index {
                    let x = (i % size[0] as usize) * *w as usize / size[0] as usize;
                    let y = (i / size[0] as usize) * *h as usize / size[1] as usize;
                    let j = 4 * (y * *w as usize + x);
                    let pair = ((usize::from(data[j]) * (rows.len() / 2 - 1) + 127) / 255) * 2;
                    let f = f32::from(data[j + 1]) / 255.;
                    std::array::from_fn(|k| rows[pair][k] * f + rows[pair + 1][k] * (1. - f))
                } else {
                    rows[0]
                };
                for (c, t) in px[..3].iter_mut().zip(tint) {
                    if t == 1. {
                        continue;
                    }
                    let v = f32::from(*c) / 255.;
                    let linear = if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    };
                    let v = (linear * t).clamp(0., 1.);
                    *c = (255.
                        * if v <= 0.0031308 {
                            12.92 * v
                        } else {
                            1.055 * v.powf(1. / 2.4) - 0.055
                        })
                    .round() as u8;
                }
            }
            return Ok(Self {
                size,
                rgba,
                srgb: true,
            });
        }
        if let Source::Solid(color) = source {
            if max_side == 0 {
                return Err(MaterialError::Unsupported(
                    "no mip fits preview texture budget",
                ));
            }
            return Ok(Self {
                size: [1, 1],
                rgba: color.to_vec(),
                srgb: false,
            });
        }
        let (size, mut rgba) = decode_tex(texture, max_side)?;
        let srgb = matches!(source, Source::Diffuse);
        if let Source::Index = source {
            let table = m
                .color_table()?
                .ok_or(MaterialError::Unsupported("missing color table"))?;
            let rows = (0..table.row_count())
                .map(|i| table.row(i).map(|r| r.diffuse()))
                .collect::<Result<Vec<_>, _>>()?;
            for px in rgba.chunks_exact_mut(4) {
                let pair = ((usize::from(px[0]) * (rows.len() / 2 - 1) + 127) / 255) * 2;
                let even = f32::from(px[1]) / 255.;
                for (k, c) in px[..3].iter_mut().enumerate() {
                    *c = (255.
                        * (rows[pair][k] * even + rows[pair + 1][k] * (1. - even)).clamp(0., 1.))
                    .round() as u8;
                }
                px[3] = 255;
            }
        }
        Ok(Self { size, rgba, srgb })
    }
}

fn decode_tex(texture: &[u8], max_side: u32) -> Result<([u32; 2], Vec<u8>), MaterialError> {
    let tex = Tex::parse(texture)?;
    if tex.layout() != Layout::D2 {
        return Err(MaterialError::Unsupported("expected a 2D color source"));
    }
    let mip = tex
        .mips()
        .find(|m| u32::from(m.width().max(m.height())) <= max_side.min(4096))
        .ok_or(MaterialError::Unsupported(
            "no mip fits preview texture budget",
        ))?;
    Ok((
        [u32::from(mip.width()), u32::from(mip.height())],
        mip.decode_rgba8(64 * 1024 * 1024)?,
    ))
}
