//! Offline authoring tool; import-action.py can also build it against a specific data checkout.
#[cfg(feature = "data-snapshot")]
#[path = "data_snapshot/binary.rs"]
mod binary;
#[cfg(feature = "data-snapshot")]
#[path = "data_snapshot/havok.rs"]
mod havok;
#[cfg(feature = "data-snapshot")]
#[path = "data_snapshot/tmb.rs"]
mod tmb;
#[cfg(not(feature = "data-snapshot"))]
use icexiv_data::tmb::{ResourceKind, Timeline, TimelineLimits};
use serde_json::{Value, json};
use std::{
    error::Error,
    path::{Component, Path},
};
#[cfg(feature = "data-snapshot")]
use tmb::{ResourceKind, Timeline, TimelineLimits};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn walk(
    read: &mut impl FnMut(&str) -> Result<Vec<u8>>,
    path: &str,
    stack: &mut Vec<String>,
    calls: &mut Vec<Value>,
    output: &mut Vec<Value>,
    clips: &mut Option<Vec<Value>>,
    props: &mut Vec<Value>,
    sounds: &mut Vec<Value>,
    budget: &mut usize,
) -> Result<()> {
    if path.is_empty()
        || !path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        || !Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("invalid logical TMB path".into());
    }
    if stack.len() >= 32 || stack.iter().any(|p| p == path) {
        return Err(format!("recursive C002 at {path}").into());
    }
    *budget = budget.checked_sub(1).ok_or("TMB expansion limit")?;
    stack.push(path.to_owned());
    let bytes = read(path)?;
    if bytes.len() > 16 << 20 {
        return Err("TMB size limit".into());
    }
    let timelines = Timeline::parse_sequence(&bytes, TimelineLimits::default())?;
    let [t] = timelines.as_slice() else {
        return Err("expected one standalone timeline".into());
    };
    walk_timeline(
        read, t, path, stack, calls, output, clips, props, sounds, budget,
    )?;
    stack.pop();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn walk_timeline(
    read: &mut impl FnMut(&str) -> Result<Vec<u8>>,
    t: &Timeline<'_>,
    path: &str,
    stack: &mut Vec<String>,
    calls: &mut Vec<Value>,
    output: &mut Vec<Value>,
    clips: &mut Option<Vec<Value>>,
    props: &mut Vec<Value>,
    sounds: &mut Vec<Value>,
    budget: &mut usize,
) -> Result<()> {
    for l in t.command_links(4 << 20, 1_000_000)? {
        let entry = &t.entries[l.command];
        if entry.tag == *b"C198" && entry.payload.len() == 32 {
            let word = |o: usize| u32::from_le_bytes(entry.payload[o..o + 4].try_into().unwrap());
            let id = (word(0) & 0xffff) as u16;
            let start = (word(0) >> 16) as i16;
            let duration = word(4) as i32;
            let slot = word(20) & 0xff;
            let model_word = word(24);
            let variant_word = word(28);
            let model_id = (model_word & 0xffff) as u16;
            let body_id = (model_word >> 16) as u16;
            let variant_id = (variant_word & 0xffff) as u16;
            if model_id != 0 && body_id != 0 {
                let actor = t.actor(l.actor)?;
                let track = t.track(l.track)?;
                let conditions = track.conditions().map(|cs| {
                    cs.map(|c| [c.operation, c.value, c.float_value.to_bits()])
                        .collect::<Vec<_>>()
                });
                calls.push(json!({"tmb":path,"actor":actor.id,"actor_start":actor.start,"ability_delay":actor.ability_delay,"track":track.id,"track_start":track.start,"conditions":conditions,"command":id,"start":start,"duration":duration}));
                let bone = if slot == 1 { "n_buki_l" } else { "n_buki_r" };
                let packed =
                    ((variant_id as u64) << 32) | ((body_id as u64) << 16) | (model_id as u64);
                props.push(json!({"chain":calls,"modelMain":packed.to_string(),"modelId":model_id,"bodyId":body_id,"variantId":variant_id,"bone":bone}));
                calls.pop();
            }
            continue;
        }
        let Some(r) = t.command_resource(l.command)? else {
            continue;
        };
        let c010 = clips.is_some() && matches!(&entry.tag, b"C010" | b"C009");
        if !c010
            && !matches!(
                r.kind,
                ResourceKind::Timeline | ResourceKind::Effect | ResourceKind::Sound
            )
        {
            continue;
        }
        let actor = t.actor(l.actor)?;
        let track = t.track(l.track)?;
        let conditions = track.conditions().map(|cs| {
            cs.map(|c| [c.operation, c.value, c.float_value.to_bits()])
                .collect::<Vec<_>>()
        });
        calls.push(json!({"tmb":path,"actor":actor.id,"actor_start":actor.start,"ability_delay":actor.ability_delay,"track":track.id,"track_start":track.start,"conditions":conditions,"command":r.id,"start":r.start,"duration":r.duration}));
        let name = std::str::from_utf8(r.name)?;
        if r.kind == ResourceKind::Timeline {
            let name = if name.ends_with(".tmb") {
                name.to_owned()
            } else {
                format!("{name}.tmb")
            };
            walk(
                read, &name, stack, calls, output, clips, props, sounds, budget,
            )?;
        } else if r.kind == ResourceKind::Sound {
            if sounds.len() < 1024 {
                sounds.push(json!({"chain":calls,"scd":name}));
            }
        } else if c010 {
            let clips = clips.as_mut().unwrap();
            if clips.len() >= 1024 {
                return Err("C010 occurrence limit".into());
            }
            clips.push(json!({"chain":calls,"name":name}));
        } else {
            if output.len() >= 1024 {
                return Err("C012 occurrence limit".into());
            }
            let fx = t.effect_command(l.command)?.ok_or("expected C012")?;
            let targets = fx
                .targets
                .map(|b| json!({"origin":b.origin,"kind":b.kind,"id":b.id}));
            output.push(json!({"chain":calls,"targets":targets,"avfx":name,"scale":fx.scale,"rotation_radians":fx.rotation_radians,"position":fx.position,"color":fx.color,"visibility":fx.visibility}));
        }
        calls.pop();
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    #[cfg(feature = "pack")]
    if args.get(1).is_some_and(|s| s == "--pack") {
        return pack::run(&args[2..]);
    }
    if !matches!(args.len(), 3 | 4) || args.get(3).is_some_and(|s| s != "--clips") {
        return Err("usage: icexiv-action EXTRACTED_ROOT LOGICAL_TMB [--clips] | --pack SQPACK ACTION_ID CAS [--actor RACE] [PAP ...]".into());
    }
    let root = Path::new(&args[1]);
    let mut events = vec![];
    let mut clips = (args.get(3).map(String::as_str) == Some("--clips")).then(Vec::new);
    let mut props = vec![];
    let mut sounds = vec![];
    walk(
        &mut |p| {
            let resource = root.join(p);
            if resource.metadata()?.len() > 16 << 20 {
                return Err("TMB size limit".into());
            }
            Ok(std::fs::read(resource)?)
        },
        &args[2],
        &mut vec![],
        &mut vec![],
        &mut events,
        &mut clips,
        &mut props,
        &mut sounds,
        &mut 1024,
    )?;
    if let Some(clips) = clips {
        serde_json::to_writer(
            std::io::stdout().lock(),
            &json!({"effects":events,"clips":clips}),
        )?;
    } else {
        serde_json::to_writer(std::io::stdout().lock(), &events)?;
    }
    Ok(())
}
#[cfg(feature = "pack")]
#[path = "pack_action.rs"]
mod pack;
