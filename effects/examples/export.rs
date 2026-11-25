//! Record the actual files read by the same loader used by the WebAssembly viewer.
//! Args: game directory, CAS directory, optional --owner (only model+rig).
//! Input: packed main model ID per line.
use anyhow::{Result, ensure};
use physis::resource::{Resource, SqPackResource};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead},
    path::PathBuf,
};

#[derive(Clone)]
struct Recording {
    game: SqPackResource,
    files: BTreeMap<String, Vec<u8>>,
}
impl Resource for Recording {
    fn read(&mut self, path: &str) -> physis::Result<Vec<u8>> {
        let bytes = self.game.read(path)?;
        self.files
            .entry(path.to_owned())
            .or_insert_with(|| bytes.clone());
        Ok(bytes)
    }
    fn exists(&mut self, path: &str) -> bool {
        self.game.exists(path)
    }
}
fn sample(
    r: &mut Recording,
    raw: u64,
    root: &std::path::Path,
    owner_only: bool,
) -> Result<Option<Value>> {
    use assets::{ImcFile, PackedModelId, weapon_vfx_imc_path};
    let key = PackedModelId::from_raw(raw);
    let imc_path = weapon_vfx_imc_path(key.model_id, key.body_id);
    if !owner_only {
        let bytes = match r.game.read(&imc_path) {
            Ok(b) => b,
            Err(physis::Error::FileNotFound { .. }) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let imc = ImcFile::parse(&bytes)?;
        if imc
            .subset_for_variant(key.variant_id)
            .first()
            .is_none_or(|e| e.vfx == 0)
        {
            return Ok(None);
        }
    }
    r.files.clear();
    let a = if owner_only {
        icexiv_effects::EffectAssets::load_owner(r, raw)?
    } else {
        icexiv_effects::EffectAssets::load(r, raw)?
    };
    let mounts = a.mounts();
    let owner = a
        .owner_path()
        .ok_or_else(|| anyhow::anyhow!("missing loaded MDL path"))?;
    let diagnostics = a.diagnostics();
    let total: usize = r.files.values().map(Vec::len).sum();
    ensure!(
        total <= 128 << 20
            && r.files.len() <= 1024
            && r.files.values().all(|v| v.len() <= 64 << 20),
        "effect bundle exceeds viewer budget"
    );
    let mut files = BTreeMap::new();
    for (name, b) in &r.files {
        let sha = format!("{:x}", Sha256::digest(b));
        let path = root.join(&sha[..2]).join(&sha[2..]);
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, b)?;
        }
        files.insert(name, sha);
    }
    Ok(Some(
        json!({"main":raw.to_string(),"owner":owner,"mounts":mounts.attachments.iter().map(|m|&m.data.avfx_path).collect::<Vec<_>>(),"files":files,"diagnostics":diagnostics}),
    ))
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game directory");
    let root = PathBuf::from(args.next().expect("CAS directory"));
    let owner_only = match args.next().as_deref() {
        None => false,
        Some("--owner") => true,
        Some(v) => anyhow::bail!("unknown option: {v}"),
    };
    let mut r = Recording {
        game: SqPackResource::from_existing(&game),
        files: BTreeMap::new(),
    };
    for line in io::stdin().lock().lines() {
        let raw = line?.parse()?;
        println!(
            "{}",
            match sample(&mut r, raw, &root, owner_only) {
                Ok(v) => json!(v),
                Err(e) => json!({"error":format!("{e:#}")}),
            }
        );
    }
    Ok(())
}
