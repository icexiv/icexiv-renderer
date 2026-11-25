//! Extra skeleton template tables: u32 count, packed (set,race) keys,
//! then the parallel array of u16 skeleton numbers.
use anyhow::{Result, ensure};
use physis::resource::Resource;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Templates(HashMap<&'static str, Option<Vec<u8>>>);

impl Templates {
    pub(super) fn lookup<R: Resource>(
        &mut self,
        source: &mut R,
        kind: &str,
        race: u16,
        set: u16,
    ) -> Result<Option<Option<String>>> {
        let (path, prefix) = match kind {
            "face" => ("chara/xls/charadb/faceSkeletonTemplate.est", 'f'),
            "hair" => ("chara/xls/charadb/hairSkeletonTemplate.est", 'h'),
            "top" => ("chara/xls/charadb/extra_top.est", 't'),
            "met" => ("chara/xls/charadb/extra_met.est", 'm'),
            _ => return Ok(None),
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = self.0.entry(path) {
            let bytes = match source.read(path) {
                Ok(b) => {
                    ensure!(b.len() >= 4, "truncated {path}");
                    let n = u32::from_le_bytes(b[..4].try_into()?) as usize;
                    ensure!(
                        n <= (1 << 20) && b.len() == 4 + 6 * n,
                        "invalid {path} length"
                    );
                    ensure!(
                        b[4..4 + 4 * n]
                            .chunks_exact(4)
                            .map(|k| u32::from_le_bytes(k.try_into().unwrap()))
                            .is_sorted(),
                        "unordered {path}"
                    );
                    Some(b)
                }
                Err(physis::Error::FileNotFound { .. }) => None,
                Err(e) => return Err(e.into()),
            };
            entry.insert(bytes);
        }
        let Some(b) = &self.0[path] else {
            return Ok(None);
        };
        let n = (b.len() - 4) / 6;
        let key = ((race as u32) << 16) | set as u32;
        let key_at = |i: usize| u32::from_le_bytes(b[4 + 4 * i..8 + 4 * i].try_into().unwrap());
        let (mut i, mut hi) = (0, n);
        while i < hi {
            let mid = i + (hi - i) / 2;
            if key_at(mid) < key {
                i = mid + 1;
            } else {
                hi = mid;
            }
        }
        if i == n || key_at(i) != key {
            return Ok(Some(None));
        }
        let o = 4 + 4 * n + 2 * i;
        let id = u16::from_le_bytes(b[o..o + 2].try_into()?);
        for j in i + 1..n {
            if key_at(j) != key {
                break;
            }
            let p = 4 + 4 * n + 2 * j;
            ensure!(
                u16::from_le_bytes(b[p..p + 2].try_into()?) == id,
                "ambiguous {path} entry ({race}, {set})"
            );
        }
        Ok(Some((id != 0).then(|| format!("chara/human/c{race:04}/skeleton/{kind}/{prefix}{id:04}/skl_c{race:04}{prefix}{id:04}.sklb"))))
    }
}
