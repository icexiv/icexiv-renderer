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
}

fn source<'a>(m: &Mtrl<'a>) -> Result<(Source, &'a str), MaterialError> {
    let mut diffuse = None;
    for i in 0..m.sampler_count() {
        let s = m.sampler(i)?;
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
            Source::Index => return Ok((role, path)),
            Source::Diffuse => diffuse = Some((role, path)),
        }
    }
    diffuse.ok_or(MaterialError::Unsupported(
        "material has no supported color source",
    ))
}

/// Logical TEX path for the supported color pass, taken from the sampler binding.
pub fn color_texture_path(bytes: &[u8]) -> Result<&str, MaterialError> {
    Ok(source(&Mtrl::parse(bytes)?)?.1)
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
        let m = Mtrl::parse(material)?;
        let (source, _) = source(&m)?;
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
        let mut rgba = mip.decode_rgba8(64 * 1024 * 1024)?;
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
        Ok(Self {
            size: [u32::from(mip.width()), u32::from(mip.height())],
            rgba,
            srgb,
        })
    }
}
