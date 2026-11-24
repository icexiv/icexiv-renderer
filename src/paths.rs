/// Ordered logical MTRL candidates for a model reference. Only continue to
/// the next candidate after confirmed absence, not an I/O or decode error.
pub fn material_paths(model: &str, reference: &str, mut variant: Option<u16>) -> Vec<String> {
    let name = reference.trim_start_matches('/');
    if name.starts_with("chara/") || name.starts_with("bgcommon/") {
        return vec![name.into()];
    }
    let Some((root, _)) = model
        .rsplit_once('/')
        .and_then(|(dir, _)| dir.rsplit_once('/'))
    else {
        return vec![];
    };
    if name.contains('/') {
        return vec![format!("{root}/material/{name}")];
    }
    let mut root = root.to_owned();
    if let Some(id) = name
        .strip_prefix("mt_")
        .and_then(|s| s.split_once('_'))
        .map(|(id, _)| id)
        && let Some(obj) = id.get(..5)
        && let Some(body) = id.get(5..10)
        && id.len() == 10
        && body.starts_with('b')
        && obj.as_bytes()[1..]
            .iter()
            .chain(&body.as_bytes()[1..])
            .all(u8::is_ascii_digit)
    {
        if obj.starts_with('w') {
            root = format!("chara/weapon/{obj}/obj/body/{body}");
        } else if obj.starts_with('c') {
            root = format!("chara/human/{obj}/obj/body/{body}");
            variant = Some(1);
        }
    }
    let mut paths = Vec::new();
    if let Some(v) = variant {
        paths.push(format!("{root}/material/v{v:04}/{name}"));
    }
    paths.push(format!("{root}/material/{name}"));
    if variant != Some(1) {
        paths.push(format!("{root}/material/v0001/{name}"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_selector_preserves_owner_and_falls_back_only_after_absence() {
        let root = "chara/weapon/w5571/obj/body/b0001/material";
        let name = "mt_w5571b0001_a.mtrl";
        assert_eq!(
            material_paths(
                "chara/weapon/w5561/obj/body/b0001/model/x.mdl",
                name,
                Some(0)
            ),
            [
                format!("{root}/v0000/{name}"),
                format!("{root}/{name}"),
                format!("{root}/v0001/{name}")
            ]
        );
        assert_eq!(
            material_paths("x/model/a.mdl", "/bgcommon/x.mtrl", Some(0)),
            ["bgcommon/x.mtrl"]
        );
    }
}
