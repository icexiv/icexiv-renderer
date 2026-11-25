//! Usage: timeline EXTRACTED_ROOT EVENTS.json PREVIEW.json FRAME_COUNT
//! Output: 60 Hz CPU samples in the same coordinates used by the GPU batch.
use anyhow::{Context, Result, ensure};
use icexiv_effects::timeline::ActionPlayer;
use std::{
    fs,
    path::{Component, Path},
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 5,
        "expected EXTRACTED_ROOT EVENTS.json PREVIEW.json FRAME_COUNT"
    );
    let (mut p, _draw) = ActionPlayer::load(
        serde_json::from_slice(&fs::read(&args[2])?)?,
        serde_json::from_slice(&fs::read(&args[3])?)?,
        |path| {
            ensure!(
                Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "invalid logical path"
            );
            fs::read(Path::new(&args[1]).join(path)).with_context(|| path.to_owned())
        },
    )?;
    for tick in 0..args[4].parse::<usize>()? {
        p.advance_to(tick as f64 / 60.)?;
        let (q, m) = p.sample();
        println!(
            "{}",
            serde_json::json!({"tick":tick,"active":p.active_occurrences().collect::<Vec<_>>(),"quads":q.len(),"meshes":m.len(),"first_position":q.first().map(|v|v.position).or_else(||m.first().map(|v|v.position))})
        );
    }
    for message in p.diagnostics {
        eprintln!("{message}");
    }
    Ok(())
}
