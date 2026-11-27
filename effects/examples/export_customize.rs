//! export_customize GAME/game OBJECTS OUT_DIR ITEM_IDS_JSON
//! Exports hairstyle & face-paint unlockable items as lightweight delta bundles
//! layered on top of the shared actor bundles.
use anyhow::{Result, ensure};
use icexiv_effects::actor::{ActorAssets, MAX_BUNDLE_BYTES, MAX_FILE_BYTES, MAX_FILES};
use physis::{
    Language,
    excel::Field,
    resource::{Resource, SqPackResource},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::Path,
};

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
            return Err(std::io::Error::other("customize resource budget exceeded").into());
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

fn field_u32(col: &Field) -> u32 {
    match *col {
        Field::UInt8(v) => u32::from(v),
        Field::UInt16(v) => u32::from(v),
        Field::UInt32(v) => v,
        Field::Int8(v) => v as u32,
        Field::Int16(v) => v as u32,
        Field::Int32(v) => v as u32,
        _ => 0,
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [game_dir, objects_dir, out_dir, item_ids_json] = args.as_slice() else {
        anyhow::bail!("usage: export_customize GAME/game OBJECTS OUT_DIR ITEM_IDS_JSON")
    };
    let item_ids: Vec<u32> = serde_json::from_str(&fs::read_to_string(item_ids_json)?)?;
    let mut game = SqPackResource::from_existing(game_dir);

    let exh = game.read_excel_sheet_header("CharaMakeCustomize")?;
    let lang = if exh.languages.contains(&Language::None) {
        Language::None
    } else if exh.languages.contains(&Language::ChineseSimplified) {
        Language::ChineseSimplified
    } else {
        exh.languages.first().copied().unwrap_or(Language::None)
    };
    let sheet = game.read_excel_sheet(&exh, "CharaMakeCustomize", lang)?;
    let mut hint_to_feature: HashMap<u32, (u8, bool)> = HashMap::new();
    for (row_id, subrows) in sheet.into_iter() {
        let Some((_, row)) = subrows.first() else {
            continue;
        };
        let feature_id = row.columns.first().map(field_u32).unwrap_or(0) as u8;
        for col in &row.columns {
            let v = field_u32(col);
            if v > 1000 && item_ids.contains(&v) && feature_id > 0 {
                let has_hair = game.exists(&format!(
                    "chara/human/c0101/obj/hair/h{feature_id:04}/model/c0101h{feature_id:04}_hir.mdl"
                )) || game.exists(&format!(
                    "chara/human/c0201/obj/hair/h{feature_id:04}/model/c0201h{feature_id:04}_hir.mdl"
                ));
                let has_decal = !assets::face_paint_decal_texture_candidates(feature_id)
                    .into_iter()
                    .all(|p| !game.exists(&p));
                let is_face_paint = if row_id >= 2000 && has_decal {
                    true
                } else if has_hair {
                    false
                } else {
                    has_decal
                };
                hint_to_feature
                    .entry(v)
                    .or_insert((feature_id, is_face_paint));
            }
        }
    }

    // Representative race/gender customize templates matching action_preview.py.
    let templates: &[(&str, [u8; 26])] = &[
        (
            "1-0",
            [
                1, 0, 1, 50, 1, 4, 1, 0, 3, 30, 103, 0, 0, 0, 1, 0, 4, 5, 2, 0, 35, 50, 0, 0, 0, 0,
            ],
        ),
        (
            "1-1",
            [
                1, 1, 1, 50, 1, 0, 1, 0, 1, 37, 54, 0, 0, 2, 0, 0, 0, 0, 0, 0, 44, 50, 0, 50, 0, 0,
            ],
        ),
        (
            "2-0",
            [
                2, 0, 1, 50, 3, 1, 1, 0, 82, 28, 52, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 2, 0, 0, 0,
            ],
        ),
        (
            "2-1",
            [
                2, 1, 1, 50, 3, 3, 1, 0, 82, 53, 43, 0, 0, 0, 0, 0, 0, 0, 0, 0, 39, 25, 0, 50, 0, 0,
            ],
        ),
        (
            "3-0",
            [
                3, 0, 1, 50, 5, 0, 1, 0, 25, 11, 29, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 1, 0, 0, 0,
            ],
        ),
        (
            "3-1",
            [
                3, 1, 1, 50, 5, 0, 1, 0, 25, 11, 182, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 1, 25, 0, 0,
            ],
        ),
        (
            "4-0",
            [
                4, 0, 1, 50, 7, 0, 1, 0, 146, 22, 94, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 25, 0, 0, 0, 0,
            ],
        ),
        (
            "4-1",
            [
                4, 1, 1, 50, 7, 2, 1, 0, 136, 81, 92, 0, 0, 0, 0, 0, 0, 0, 1, 1, 42, 25, 0, 50, 0,
                0,
            ],
        ),
        (
            "5-0",
            [
                5, 0, 1, 50, 9, 1, 1, 0, 25, 13, 165, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 25, 0, 0, 0, 0,
            ],
        ),
        (
            "5-1",
            [
                5, 1, 1, 50, 9, 3, 1, 0, 42, 47, 177, 0, 0, 0, 0, 0, 0, 0, 0, 3, 44, 25, 0, 50, 0,
                0,
            ],
        ),
        (
            "6-0",
            [
                6, 0, 1, 50, 11, 1, 1, 0, 109, 150, 183, 0, 0, 138, 0, 0, 0, 0, 0, 0, 0, 50, 0, 0,
                0, 0,
            ],
        ),
        (
            "6-1",
            [
                6, 1, 1, 50, 11, 1, 1, 0, 16, 90, 90, 0, 0, 36, 0, 1, 4, 0, 1, 0, 43, 50, 0, 50, 0,
                0,
            ],
        ),
        (
            "7-0",
            [
                7, 0, 1, 50, 13, 1, 1, 0, 27, 43, 46, 0, 0, 7, 3, 0, 2, 2, 2, 2, 0, 50, 0, 0, 0, 0,
            ],
        ),
    ];

    fs::create_dir_all(out_dir)?;
    let mut index = BTreeMap::new();
    let mut all_objects = BTreeMap::new();

    for &item_id in &item_ids {
        let Some(&(feature_id, is_face_paint)) = hint_to_feature.get(&item_id) else {
            continue;
        };
        let mut files = BTreeMap::new();
        let mut supported_actors = Vec::new();
        if is_face_paint {
            for cand in assets::face_paint_decal_texture_candidates(feature_id) {
                if let Ok(bytes) = game.read(&cand) {
                    let sha = store(Path::new(objects_dir), &bytes)?;
                    files.insert(cand, sha.clone());
                    all_objects.insert(sha, ());
                    break;
                }
            }
            if files.is_empty() {
                continue;
            }
            for (key, _) in templates {
                supported_actors.push(*key);
            }
        } else {
            for (key, base) in templates {
                let mut cust = *base;
                cust[6] = feature_id;
                let mut rec = Recording {
                    game: game.clone(),
                    files: BTreeMap::new(),
                    bytes: 0,
                    exceeded: false,
                };
                let Ok(customize) = assets::CharacterCustomize::from_bytes(&cust) else {
                    continue;
                };
                let req = assets::DressedCharacterLoadRequest::new(customize, "hair");
                if ActorAssets::load(&mut rec, &req).is_ok() && !rec.exceeded {
                    let mut found_hair = false;
                    for (path, bytes) in &rec.files {
                        if path.contains("/obj/hair/") || path.contains("/skeleton/hair/") {
                            let sha = store(Path::new(objects_dir), bytes)?;
                            files.insert(path.clone(), sha.clone());
                            all_objects.insert(sha, ());
                            found_hair = true;
                        }
                    }
                    if found_hair {
                        supported_actors.push(*key);
                    }
                }
            }
            if files.is_empty() {
                continue;
            }
        }

        let manifest = json!({
            "kind": if is_face_paint { "facepaint" } else { "hairstyle" },
            "featureId": feature_id,
            "actors": supported_actors,
            "files": files,
            "objects": "../objects/sha256/",
        });
        let out_path = Path::new(out_dir).join(format!("{item_id}.json"));
        fs::write(&out_path, serde_json::to_string(&manifest)?)?;
        index.insert(
            item_id.to_string(),
            format!("models/customize/{item_id}.json"),
        );
    }

    let summary = json!({
        "items": index,
        "objects": all_objects.keys().collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string(&summary)?);
    ensure!(!index.is_empty(), "no customize items resolved");
    Ok(())
}
