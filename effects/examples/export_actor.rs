//! export_actor GAME/game OBJECTS actor-request.json > actor.json
use anyhow::{Context, Result, ensure};
use icexiv_effects::actor::{
    ActorAssets, ActorRequest, MAX_BUNDLE_BYTES, MAX_FILE_BYTES, MAX_FILES,
};
use physis::resource::{Resource, SqPackResource};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
};

#[derive(Deserialize)]
struct Request {
    #[serde(flatten)]
    actor: ActorRequest,
    pap: String,
    record: u16,
}

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
            return Err(std::io::Error::other("actor resource budget exceeded").into());
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
    if path.exists() {
        ensure!(
            fs::metadata(&path)?.len() == bytes.len() as u64 && fs::read(&path)? == bytes,
            "existing CAS object is corrupt: {}",
            path.display()
        );
    } else {
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
    let [game, objects, input] = args.as_slice() else {
        anyhow::bail!("usage: export_actor GAME/game OBJECTS REQUEST.json")
    };
    let mut json = Vec::new();
    fs::File::open(input)?.take(65537).read_to_end(&mut json)?;
    ensure!(json.len() <= 65536, "oversized actor request");
    let request: Request = serde_json::from_slice(&json)?;
    let req = request.actor.dressed()?;
    let sklb = assets::character_skeleton_path(req.customize.race_code());
    ensure!(
        request.pap.starts_with("chara/")
            && request.pap.ends_with(".pap")
            && request
                .pap
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "expected logical character PAP path"
    );
    let mut source = Recording {
        game: SqPackResource::from_existing(game),
        files: Default::default(),
        bytes: 0,
        exceeded: false,
    };
    let actor = ActorAssets::load(&mut source, &req)?;
    ensure!(
        source.files.contains_key(&sklb),
        "base skeleton was not loaded: {sklb}"
    );
    let pap = source.read(&request.pap)?;
    ensure!(!source.exceeded, "actor resource budget exceeded");
    // Use the authored record, not the index of the successfully decoded clips
    // (the latter skips unsupported bindings). Resolve it to a playable name.
    let clips = assets::load_animation_set_from_pap_bytes(&pap, &actor.skeleton)?;
    ensure!(
        usize::from(request.record) < usize::from(u16::from_le_bytes(pap[8..10].try_into()?)),
        "PAP record out of range"
    );
    let start =
        u32::from_le_bytes(pap[14..18].try_into()?) as usize + 40 * usize::from(request.record);
    let entry = pap.get(start..start + 32).context("truncated PAP record")?;
    let name = std::str::from_utf8(entry.split(|&b| b == 0).next().unwrap_or(entry))?;
    ensure!(
        clips.animations.iter().any(|a| a.name == name),
        "PAP record {name} is not playable"
    );
    let files = source
        .files
        .iter()
        .map(|(p, b)| Ok((p, store(Path::new(objects), b)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let manifest = serde_json::json!({"customize":request.actor.customize,"equipment":request.actor.equipment,
        "files":files,"sklb":sklb,"pap":request.pap,"record":request.record});
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
