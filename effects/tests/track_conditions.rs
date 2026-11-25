use anyhow::Result;
use icexiv_effects::timeline::{ActionPlayer, Event, NamedClip, Preview, clip_windows};
use serde_json::{Value, json};

#[test]
fn stone_motion_and_effect_admission_share_actor_values_and_explicit_override() -> Result<()> {
    let path = "chara/action/battle/magic_cnj_end.tmb";
    let links = [
        (
            3,
            json!([
                [1, 5, 0],
                [19, 268435497, 0],
                [13, 0, 0],
                [17, 35, 0],
                [2, 4, 0],
                [13, 0, 0],
                [21, 0, 0],
                [2, 1, 0]
            ]),
        ),
        (
            4,
            json!([
                [1, 1, 0],
                [19, 268435497, 0],
                [13, 0, 0],
                [17, 35, 0],
                [2, 1, 0]
            ]),
        ),
    ]
    .map(|(track, conditions)| {
        json!({"tmb":path,"track":track,"actor":2,"actor_start":0,"track_start":0,
        "command":track,"start":0,"duration":30,"ability_delay":0,"conditions":conditions})
    });
    let mut config: Value = serde_json::from_str(include_str!("../examples/stone-preview.json"))?;
    config["condition_inputs"] =
        json!({"variables":{"268435497":1},"actors":{(path):{"2":{"268435497":35}}}});
    for (enabled, expected) in [(json!([]), 1), (json!([[path, 3]]), 0)] {
        config["enabled_tracks"] = enabled;
        let preview = || serde_json::from_value::<Preview>(config.clone());
        let clips: Vec<NamedClip> = serde_json::from_value(json!([
            {"name":"primary","chain":[links[0]]},{"name":"offhand","chain":[links[1]]}]))?;
        let (windows, _) = clip_windows(clips, &preview()?)?;
        assert_eq!(
            windows.iter().map(|w| w.occurrence).collect::<Vec<_>>(),
            [expected]
        );
        let events:Vec<Event>=links.iter().map(|link|serde_json::from_value(json!({
            "chain":[link],"avfx":"test.avfx","scale":[1,1,1],"rotation_radians":[0,0,0],"position":[0,0,0],
            "color":[1,1,1,1],"visibility":0,"targets":[{"origin":1,"kind":1,"id":-1},{"origin":2,"kind":0,"id":-1}]
        }))).collect::<Result<_,_>>()?;
        let (mut p, _) = ActionPlayer::load(events, preview()?, |_| Ok(b"XFVA\0\0\0\0".to_vec()))?;
        p.advance_to(0.)?;
        assert_eq!(p.active_occurrences().collect::<Vec<_>>(), [expected]);
    }
    Ok(())
}
