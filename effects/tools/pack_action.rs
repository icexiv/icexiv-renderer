use super::{Result, walk};
use icexiv_data::{
    avfx::Avfx,
    excel::{Exd, Exh, Language},
    pap::Pap,
    sqpack::{SqPack, local::Local},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

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
        return Err("expected SQPACK ACTION_ID CAS [logical PAP paths ...]".into());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let sq = SqPack::new(rt.block_on(Local::new(&args[0]))?).with_max_read_bytes(Some(64 << 20));
    let mut read = |p: &str| -> Result<Vec<u8>> {
        Ok(rt.block_on(sq.read(p)).map_err(|e| format!("{p}: {e}"))?)
    };
    let id = args[1].parse()?;
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
    let row =
        u32::try_from(row).map_err(|_| format!("Action {id} has no nonnegative timeline ID"))?;
    let (h, d) = sheet(&mut read, "actiontimeline", row)?;
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
    let entry = format!("chara/action/{key}.tmb");
    let mut events = vec![];
    let mut clips = Some(vec![]);
    walk(
        &mut read,
        &entry,
        &mut vec![],
        &mut vec![],
        &mut events,
        &mut clips,
        &mut 1024,
    )?;
    let clips = clips.unwrap();
    let mut bundle = Bundle {
        cas: Path::new(&args[2]),
        files: BTreeMap::new(),
        size: 0,
    };
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
    let mut paps = BTreeMap::new();
    let mut packages = args[3..].to_vec();
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
        for r in Pap::parse_with_limits(
            &b,
            icexiv_data::havok::Limits {
                max_values: 8 << 20,
                ..Default::default()
            },
        )?
        .records()
        {
            let n = std::str::from_utf8(r.name)?;
            if clips.iter().any(|c| c["name"] == n) {
                if paps.insert(n.to_owned(), path).is_some() {
                    return Err(format!("ambiguous PAP record: {n}").into());
                }
                matched = true;
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
    let unresolved: Vec<_> = clips
        .iter()
        .filter_map(|c| c["name"].as_str())
        .filter(|n| !paps.contains_key(*n))
        .collect();
    let manifest: Value = json!({"action":id,"timeline":row,"entry":entry,"effects":events,"clips":clips,
        "files":bundle.files,"sklb":sklb,"paps":paps,"unresolvedClips":unresolved});
    serde_json::to_writer(std::io::stdout().lock(), &manifest)?;
    Ok(())
}
