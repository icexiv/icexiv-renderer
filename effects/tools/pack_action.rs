use super::{Result, Timeline, TimelineLimits, walk, walk_timeline};
use icexiv_data::{
    avfx::Avfx,
    excel::{Exd, Exh, Language},
    imc::Imc,
    mdl::Mdl,
    mtrl::Mtrl,
    pap::Pap,
    sqpack::{SqPack, local::Local},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

fn timeline_key(read: &mut impl FnMut(&str) -> Result<Vec<u8>>, row: u32) -> Result<String> {
    let (h, d) = sheet(read, "actiontimeline", row)?;
    let table = Exd::parse(&d, &h)?;
    let timeline = table.row(row)?.ok_or("action timeline not found")?;
    #[cfg(not(feature = "data-snapshot"))]
    let key = timeline.action_timeline_key()?;
    #[cfg(feature = "data-snapshot")]
    let key = match timeline.field(6)? {
        icexiv_data::excel::Value::String(key)
            if timeline.subrow_id().is_none()
                && h.fixed_row_size() == 24
                && h.columns().len() == 22
                && h.columns()[6].offset == 0 =>
        {
            key
        }
        _ => return Err("unsupported ActionTimeline key column".into()),
    };
    let key = std::str::from_utf8(key)?;
    if key.is_empty() {
        return Err("ActionTimeline key is empty".into());
    }
    Ok(key.to_owned())
}

fn shift_chain_start(item: &mut Value, offset: i64) {
    if offset == 0 {
        return;
    }
    if let Some(first) = item
        .get_mut("chain")
        .and_then(Value::as_array_mut)
        .and_then(|a| a.first_mut())
        && let Some(obj) = first.as_object_mut()
    {
        let cur = obj.get("actor_start").and_then(Value::as_i64).unwrap_or(0);
        obj.insert("actor_start".into(), json!(cur + offset));
    }
}

fn phase_duration(clips: &[Value]) -> i64 {
    clips
        .iter()
        .filter_map(|c| c.get("chain")?.as_array()?.last())
        .map(|l| {
            let s = l.get("start").and_then(Value::as_i64).unwrap_or(0)
                + l.get("track_start").and_then(Value::as_i64).unwrap_or(0)
                + l.get("actor_start").and_then(Value::as_i64).unwrap_or(0);
            let d = l.get("duration").and_then(Value::as_i64).unwrap_or(0);
            (s + d).max(0)
        })
        .max()
        .unwrap_or(0)
}

fn store_weapon_prop(
    sq: &SqPack<Local>,
    rt: &tokio::runtime::Runtime,
    bundle: &mut Bundle<'_>,
    model_id: u16,
    body_id: u16,
    variant_id: u16,
) -> Result<()> {
    let try_read = |p: &str| -> Result<Option<Vec<u8>>> {
        match rt.block_on(sq.read(p)) {
            Ok(b) => Ok(Some(b)),
            Err(icexiv_data::sqpack::Error::MissingResource(_)) => Ok(None),
            Err(e) => Err(format!("{p}: {e}").into()),
        }
    };
    let root = format!("chara/weapon/w{model_id:04}/obj/body/b{body_id:04}");
    let mdl_path = format!("{root}/model/w{model_id:04}b{body_id:04}.mdl");
    let Some(mdl_bytes) = try_read(&mdl_path)? else {
        return Ok(());
    };
    bundle.store(&mdl_path, &mdl_bytes)?;
    let sk_path =
        format!("chara/weapon/w{model_id:04}/skeleton/base/b0001/skl_w{model_id:04}b0001.sklb");
    if let Some(sk) = try_read(&sk_path)? {
        bundle.store(&sk_path, &sk)?;
    }
    let imc_path = format!("{root}/b{body_id:04}.imc");
    let mut mat_variant = variant_id.max(1);
    if let Some(imc_bytes) = try_read(&imc_path)? {
        bundle.store(&imc_path, &imc_bytes)?;
        if let Ok(imc) = Imc::parse(&imc_bytes)
            && let Ok(entry) = imc.entry(variant_id, 0)
        {
            mat_variant = u16::from(entry.material_id).max(1);
        }
    }
    let mdl = Mdl::parse(&mdl_bytes)?;
    for i in 0..mdl.material_count() {
        let name = std::str::from_utf8(mdl.material(i)?)?.trim_start_matches('/');
        let candidates = if name.starts_with("chara/") || name.starts_with("bgcommon/") {
            vec![name.to_owned()]
        } else {
            vec![
                format!("{root}/material/v{mat_variant:04}/{name}"),
                format!("{root}/material/{name}"),
                format!("{root}/material/v0001/{name}"),
            ]
        };
        for mtrl_path in candidates {
            if let Some(mtrl_bytes) = try_read(&mtrl_path)? {
                bundle.store(&mtrl_path, &mtrl_bytes)?;
                let mtrl = Mtrl::parse(&mtrl_bytes)?;
                for t in 0..mtrl.texture_count() {
                    let tp = std::str::from_utf8(mtrl.texture(t)?.path)?;
                    if !tp.is_empty()
                        && !bundle.files.contains_key(tp)
                        && let Some(tb) = try_read(tp)?
                    {
                        bundle.store(tp, &tb)?;
                    }
                }
                let shpk = std::str::from_utf8(mtrl.shader_package()?)?;
                let shpk_path = format!("shader/sm5/shpk/{shpk}");
                if !bundle.files.contains_key(&shpk_path)
                    && let Some(sb) = try_read(&shpk_path)?
                {
                    bundle.store(&shpk_path, &sb)?;
                }
                break;
            }
        }
    }
    Ok(())
}

fn sheet(
    read: &mut impl FnMut(&str) -> Result<Vec<u8>>,
    name: &str,
    id: u32,
) -> Result<(Exh, Vec<u8>)> {
    let h = Exh::parse(&read(&format!("exd/{name}.exh"))?)?;
    let p = h.page_for_row(id).ok_or("missing sheet page")?;
    let lang = h.languages().first().copied().unwrap_or(Language::None);
    let path = format!("exd/{name}_{}{}.exd", p.start_id, lang.suffix());
    Ok((h, read(&path)?))
}

struct Bundle<'a> {
    cas: &'a Path,
    files: BTreeMap<String, String>,
    size: usize,
}
impl Bundle<'_> {
    fn store(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        if self.files.contains_key(name) {
            return Ok(());
        }
        self.size += bytes.len();
        if self.size > 128 << 20 || self.files.len() >= 1024 || bytes.len() > 64 << 20 {
            return Err("action bundle exceeds viewer budget".into());
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        let path = self.cas.join(&digest[..2]).join(&digest[2..]);
        fs::create_dir_all(path.parent().unwrap())?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => std::io::Write::write_all(&mut f, bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(path)? != bytes {
                    return Err("existing object differs from its digest".into());
                }
            }
            Err(e) => return Err(e.into()),
        }
        self.files.insert(name.into(), digest);
        Ok(())
    }
}

/// Resolve a sheet ID to its TMB and archive the exact resources the preview consumes.
pub(super) fn run(args: &[String]) -> Result<()> {
    if args.len() < 3 {
        return Err(
            "expected SQPACK (ACTION_ID | --timeline TIMELINE_ID) CAS [--actor RACE] [PAP ...]"
                .into(),
        );
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let sq = SqPack::new(rt.block_on(Local::new(&args[0]))?).with_max_read_bytes(Some(64 << 20));
    let mut read = |p: &str| -> Result<Vec<u8>> {
        Ok(rt.block_on(sq.read(p)).map_err(|e| format!("{p}: {e}"))?)
    };
    // Emote items reference ActionTimeline rows directly; live actions go
    // through the Action sheet first.
    let (id, rows, cas_index) = if args[1] == "--timeline" {
        let row: u32 = args.get(2).ok_or("missing timeline ID")?.parse()?;
        (None, vec![row], 3)
    } else if args[1] == "--timelines" {
        let raw = args.get(2).ok_or("missing timeline IDs")?;
        let rows: Vec<u32> = raw
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u32>())
            .collect::<std::result::Result<_, _>>()?;
        if rows.is_empty() {
            return Err("empty --timelines list".into());
        }
        (None, rows, 3)
    } else {
        let id: u32 = args[1].parse()?;
        let (h, d) = sheet(&mut read, "action", id)?;
        let actions = Exd::parse(&d, &h)?;
        let action = actions.row(id)?.ok_or("action not found")?;
        #[cfg(not(feature = "data-snapshot"))]
        let row = action.action_timeline_id()?;
        #[cfg(feature = "data-snapshot")]
        let row = match action.field(7)? {
            icexiv_data::excel::Value::Int16(id)
                if action.subrow_id().is_none()
                    && h.fixed_row_size() == 64
                    && h.columns().len() == 71
                    && h.columns()[7].offset == 32 =>
            {
                id
            }
            _ => return Err("unsupported Action timeline column".into()),
        };
        let row = u32::try_from(row)
            .map_err(|_| format!("Action {id} has no nonnegative timeline ID"))?;
        (Some(id), vec![row], 2)
    };
    let mut events = vec![];
    let mut clips = vec![];
    let mut props = vec![];
    let mut sounds = vec![];
    let mut phases = vec![];
    let mut entries = vec![];
    let mut keys = vec![];
    let mut cursor = 0i64;
    for &r in &rows {
        let key = timeline_key(&mut read, r)?;
        let entry = format!("chara/action/{key}.tmb");
        let mut ph_events = vec![];
        let mut ph_clips = Some(vec![]);
        let mut ph_props = vec![];
        let mut ph_sounds = vec![];
        walk(
            &mut read,
            &entry,
            &mut vec![],
            &mut vec![],
            &mut ph_events,
            &mut ph_clips,
            &mut ph_props,
            &mut ph_sounds,
            &mut 1024,
        )?;
        let mut ph_clips = ph_clips.unwrap();
        let dur = phase_duration(&ph_clips);
        for item in ph_clips
            .iter_mut()
            .chain(ph_events.iter_mut())
            .chain(ph_props.iter_mut())
            .chain(ph_sounds.iter_mut())
        {
            shift_chain_start(item, cursor);
        }
        let phase_kind = if key.ends_with("_start") {
            "start"
        } else if key.ends_with("_loop") {
            "loop"
        } else if key.ends_with("_end") {
            "end"
        } else {
            "main"
        };
        phases.push(json!({"kind": phase_kind, "timeline": r, "key": key, "start": cursor, "end": cursor + dur}));
        cursor += dur;
        clips.extend(ph_clips);
        events.extend(ph_events);
        props.extend(ph_props);
        sounds.extend(ph_sounds);
        entries.push(entry);
        keys.push(key);
    }
    let row = rows[0];
    let entry = entries[0].clone();
    let key = keys[0].as_str();
    let mut bundle = Bundle {
        cas: Path::new(&args[cas_index]),
        files: BTreeMap::new(),
        size: 0,
    };
    let mut paps = BTreeMap::new();
    let mut packages = args[cas_index + 1..].to_vec();
    let mut sklb = None;
    let probe = packages.first().is_some_and(|a| a == "--actor");
    let mut discovery = false;
    if probe {
        let race = packages
            .get(1)
            .ok_or("missing --actor race code")?
            .parse::<u16>()?;
        packages.drain(..2);
        let base = format!("chara/human/c{race:04}");
        let sk = format!("{base}/skeleton/base/b0001/skl_c{race:04}b0001.sklb");
        bundle.store(&sk, &read(&sk)?)?;
        sklb = Some(sk);
        if packages.is_empty() && !clips.is_empty() {
            discovery = true;
            let animation = format!("{base}/animation/a0001");
            // Mirror each reachable clip's actual command path in the
            // weapon-style tree. Presence is only a probe; the PAP record name
            // below is authoritative. A child TMB can supply a second package.
            let style = key
                .split('/')
                .find(|part| part.starts_with("bt_"))
                .unwrap_or("bt_common");
            for clip in &clips {
                let chain = clip["chain"].as_array().ok_or("invalid C010 chain")?;
                for link in chain.iter().rev() {
                    if let Some(k) = link["tmb"]
                        .as_str()
                        .and_then(|p| p.strip_prefix("chara/action/"))
                        .and_then(|p| p.strip_suffix(".tmb"))
                    {
                        let path = format!("{animation}/{style}/{k}.pap");
                        if !packages.contains(&path) {
                            packages.push(path);
                        }
                    }
                }
            }
            packages.push(format!("{animation}/bt_common/resident/action.pap"));
            if packages.len() > 128 {
                return Err("PAP candidate limit; supply explicit paths".into());
            }
        }
    }
    for path in &packages {
        let b = match rt.block_on(sq.read(path)) {
            Ok(b) => b,
            Err(icexiv_data::sqpack::Error::MissingResource(_)) if discovery => continue,
            Err(e) => return Err(format!("{path}: {e}").into()),
        };
        let mut matched = false;
        let parsed = Pap::parse_with_limits(
            &b,
            icexiv_data::havok::Limits {
                max_values: 8 << 20,
                ..Default::default()
            },
        )?;
        let pap_timelines = Timeline::parse_sequence(parsed.timeline, TimelineLimits::default())?;
        for (rec_idx, r) in parsed.records().enumerate() {
            let n = std::str::from_utf8(r.name)?;
            if clips.iter().any(|c| c["name"] == n) {
                if paps.insert(n.to_owned(), path).is_some() {
                    return Err(format!("ambiguous PAP record: {n}").into());
                }
                matched = true;
                if let Some(tl) = pap_timelines.get(rec_idx) {
                    for clip in clips.iter().filter(|c| c["name"] == n) {
                        let mut calls = clip["chain"]
                            .as_array()
                            .ok_or("invalid C010 chain")?
                            .clone();
                        walk_timeline(
                            &mut read,
                            tl,
                            path,
                            &mut vec![path.clone()],
                            &mut calls,
                            &mut events,
                            &mut None,
                            &mut props,
                            &mut sounds,
                            &mut 1024,
                        )?;
                    }
                }
            }
        }
        if !matched {
            if discovery {
                continue;
            }
            return Err(format!("{path} supplies no requested C010 clip").into());
        }
        bundle.store(path, &b)?;
    }
    for e in &events {
        let path = e["avfx"].as_str().ok_or("missing AVFX name")?;
        if bundle.files.contains_key(path) {
            continue;
        }
        let b = read(path)?;
        bundle.store(path, &b)?;
        for t in Avfx::parse(&b)?.textures {
            let t = std::str::from_utf8(t)?;
            if !t.is_empty() && !bundle.files.contains_key(t) {
                bundle.store(t, &read(t)?)?;
            }
        }
    }
    for p in &props {
        if let (Some(mid), Some(bid), Some(vid)) = (
            p.get("modelId").and_then(Value::as_u64),
            p.get("bodyId").and_then(Value::as_u64),
            p.get("variantId").and_then(Value::as_u64),
        ) {
            store_weapon_prop(&sq, &rt, &mut bundle, mid as u16, bid as u16, vid as u16)?;
        }
    }
    let unresolved: Vec<_> = clips
        .iter()
        .filter_map(|c| c["name"].as_str())
        .filter(|n| !paps.contains_key(*n))
        .collect();
    let manifest: Value = json!({"action":id,"timeline":row,"entry":entry,"phases":phases,"effects":events,"clips":clips,"props":props,"sounds":sounds,
        "files":bundle.files,"sklb":sklb,"paps":paps,"unresolvedClips":unresolved});
    serde_json::to_writer(std::io::stdout().lock(), &manifest)?;
    Ok(())
}
