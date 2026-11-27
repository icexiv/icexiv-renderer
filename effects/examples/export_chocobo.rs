//! export_chocobo GAME/game OBJECTS > chocobo.json
use anyhow::{Result, ensure};
use assets::{CharaModelKind, CharaModelLoadRequest, CharaModelType, PackedCharaModelId};
use icexiv_effects::actor::{MAX_BUNDLE_BYTES, MAX_FILE_BYTES, MAX_FILES};
use physis::resource::{Resource, SqPackResource};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

#[derive(Clone)]
struct Recording {
    game: SqPackResource,
    files: BTreeMap<String, Vec<u8>>,
    bytes: usize,
    exceeded: bool,
}
impl Resource for Recording {
    fn read(&mut self, path: &str) -> physis::Result<Vec<u8>> {
        if let Some(bytes) = self.files.get(path) {
            return Ok(bytes.clone());
        }
        let bytes = self.game.read(path)?;
        if self.exceeded
            || bytes.len() > MAX_FILE_BYTES
            || self.files.len() >= MAX_FILES
            || self.bytes + bytes.len() > MAX_BUNDLE_BYTES
        {
            self.exceeded = true;
            return Err(std::io::Error::other("chocobo resource budget exceeded").into());
        }
        self.bytes += bytes.len();
        self.files.insert(path.into(), bytes.clone());
        Ok(bytes)
    }
    fn exists(&mut self, path: &str) -> bool {
        self.game.exists(path)
    }
}

fn store(root: &Path, bytes: &[u8]) -> Result<String> {
    let hash = format!("{:x}", Sha256::digest(bytes));
    let dir = root.join(&hash[..2]);
    let path = dir.join(&hash[2..]);
    if !path.exists() {
        fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!(".{}-{}", std::process::id(), hash));
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        let result = (|| -> Result<()> {
            f.write_all(bytes)?;
            drop(f);
            fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result?;
    }
    Ok(hash)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [game, objects] = args.as_slice() else {
        anyhow::bail!("usage: export_chocobo GAME/game OBJECTS")
    };
    let mut rec = Recording {
        game: SqPackResource::from_existing(game),
        files: BTreeMap::new(),
        bytes: 0,
        exceeded: false,
    };
    let req = CharaModelLoadRequest {
        item_id: 0,
        item_name: "chocobo".into(),
        kind: CharaModelKind::Mount,
        model: PackedCharaModelId {
            model_id: 1,
            base_id: 1,
            variant_id: 1,
            chara_type: CharaModelType::Demihuman,
        },
    };
    let (_data, _skel) = assets::load_chara_model_with_skeleton_from_resource(&mut rec, &req)?;
    ensure!(!rec.exceeded, "chocobo resource budget exceeded");
    let mut manifest_files = BTreeMap::new();
    for (path, bytes) in &rec.files {
        manifest_files.insert(path.clone(), store(Path::new(objects), bytes)?);
    }
    let out = serde_json::json!({ "files": manifest_files });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}
