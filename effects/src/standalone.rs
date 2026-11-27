//! Standalone and Chocobo-host models rendered inside the unified EffectPlayer scene.
use super::EffectPlayer;
use anyhow::{Context, Result};
use assets::{
    CharaModelKind, CharaModelLoadRequest, CharaModelType, EquipmentModelLoadRequest,
    FurnitureModelKind, FurnitureModelLoadRequest, PackedCharaModelId, WeaponModelLoadRequest,
};
use physis::resource::Resource;
use scene::{ModelInstance, PreparedModelOptions};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StandaloneSpec {
    #[serde(rename_all = "camelCase")]
    Equipment {
        model_main: String,
        #[serde(default)]
        model_sub: Option<String>,
        equip_slot_category: u32,
        #[serde(default = "default_race_id")]
        race_id: u16,
        #[serde(default)]
        stain_ids: [u8; 2],
        #[serde(default)]
        item_id: u32,
    },
    #[serde(rename_all = "camelCase")]
    Weapon {
        model_main: String,
        #[serde(default)]
        model_sub: Option<String>,
        #[serde(default)]
        stain_ids: [u8; 2],
        #[serde(default)]
        item_id: u32,
    },
    #[serde(rename_all = "camelCase")]
    Chara {
        #[serde(default)]
        chara_kind: String,
        model_id: u16,
        base_id: u16,
        #[serde(default = "default_one")]
        variant_id: u16,
        #[serde(default)]
        is_demihuman: bool,
    },
    #[serde(rename_all = "camelCase")]
    Furniture {
        #[serde(default)]
        furniture_kind: String,
        model_key: u16,
    },
    #[serde(rename_all = "camelCase")]
    Chocobo {
        #[serde(default = "default_one")]
        head_id: u16,
        #[serde(default = "default_one")]
        body_id: u16,
        #[serde(default = "default_one")]
        feet_id: u16,
        #[serde(default = "default_one")]
        head_variant: u16,
        #[serde(default = "default_one")]
        body_variant: u16,
        #[serde(default = "default_one")]
        feet_variant: u16,
    },
}

fn default_race_id() -> u16 {
    101
}
fn default_one() -> u16 {
    1
}

/// Resource wrapper that restricts demihuman slot `.mdl` probing to one slot suffix.
#[derive(Clone)]
struct SlotFilter<R> {
    inner: R,
    allow_suffix: &'static str,
}

impl<R: Resource + Clone> Resource for SlotFilter<R> {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, physis::Error> {
        if path.ends_with(".mdl") && !path.ends_with(self.allow_suffix) {
            return Err(physis::Error::FileNotFound {
                path: path.to_string(),
            });
        }
        self.inner.read(path)
    }

    fn exists(&mut self, path: &str) -> bool {
        if path.ends_with(".mdl") && !path.ends_with(self.allow_suffix) {
            return false;
        }
        self.inner.exists(path)
    }
}

impl EffectPlayer {
    /// Load a standalone model (equipment, weapon, mount, minion, furniture, or dressed chocobo)
    /// into the unified PBR + VFX scene.
    pub fn set_standalone<R: Resource + Clone>(
        &mut self,
        source: &mut R,
        spec: &StandaloneSpec,
    ) -> Result<()> {
        let opts = PreparedModelOptions::default().with_component_preview_layout(false);
        let mut instances: Vec<ModelInstance> = Vec::new();
        match spec {
            StandaloneSpec::Equipment {
                model_main,
                model_sub,
                equip_slot_category,
                race_id,
                stain_ids,
                item_id,
            } => {
                let req = EquipmentModelLoadRequest {
                    item_id: *item_id,
                    item_name: String::new(),
                    model_main: model_main.parse().context("invalid equipment modelMain")?,
                    model_sub: model_sub
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .context("invalid equipment modelSub")?,
                    equip_slot_category: *equip_slot_category,
                    race_id: *race_id,
                    stain_ids: *stain_ids,
                };
                let data = assets::load_equipment_model_from_resource(source, &req)?;
                instances.push(
                    self.renderer
                        .context()
                        .create_model_with_skeleton(&data, opts, None),
                );
            }
            StandaloneSpec::Weapon {
                model_main,
                model_sub,
                stain_ids,
                item_id,
            } => {
                let req = WeaponModelLoadRequest {
                    item_id: *item_id,
                    item_name: String::new(),
                    model_main: model_main.parse().context("invalid weapon modelMain")?,
                    model_sub: model_sub
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .context("invalid weapon modelSub")?,
                    stain_ids: *stain_ids,
                };
                let data = assets::load_weapon_model_from_resource_request(source, &req)?;
                instances.push(
                    self.renderer
                        .context()
                        .create_model_with_skeleton(&data, opts, None),
                );
            }
            StandaloneSpec::Chara {
                chara_kind,
                model_id,
                base_id,
                variant_id,
                is_demihuman,
            } => {
                let kind = if chara_kind == "mount" {
                    CharaModelKind::Mount
                } else {
                    CharaModelKind::Minion
                };
                let chara_type = if *is_demihuman {
                    CharaModelType::Demihuman
                } else {
                    CharaModelType::Monster
                };
                let req = CharaModelLoadRequest {
                    item_id: 0,
                    item_name: String::new(),
                    kind,
                    model: PackedCharaModelId {
                        model_id: *model_id,
                        base_id: *base_id,
                        variant_id: (*variant_id).max(1),
                        chara_type,
                    },
                };
                let (data, skeleton) =
                    assets::load_chara_model_with_skeleton_from_resource(source, &req)?;
                instances.push(self.renderer.context().create_model_with_skeleton(
                    &data,
                    opts,
                    skeleton.as_ref(),
                ));
            }
            StandaloneSpec::Furniture {
                furniture_kind,
                model_key,
            } => {
                let kind = if furniture_kind == "outdoor" {
                    FurnitureModelKind::Outdoor
                } else {
                    FurnitureModelKind::Indoor
                };
                let req = FurnitureModelLoadRequest {
                    item_id: 0,
                    item_name: String::new(),
                    kind,
                    model_key: *model_key,
                };
                let data = assets::load_furniture_model_from_resource(source, &req)?;
                instances.push(
                    self.renderer
                        .context()
                        .create_model_with_skeleton(&data, opts, None),
                );
            }
            StandaloneSpec::Chocobo {
                head_id,
                body_id,
                feet_id,
                head_variant,
                body_variant,
                feet_variant,
            } => {
                let slots = [
                    ("_met.mdl", (*head_id).max(1), (*head_variant).max(1)),
                    ("_top.mdl", (*body_id).max(1), (*body_variant).max(1)),
                    ("_sho.mdl", (*feet_id).max(1), (*feet_variant).max(1)),
                ];
                for (suffix, base_id, variant_id) in slots {
                    let req = CharaModelLoadRequest {
                        item_id: 0,
                        item_name: String::new(),
                        kind: CharaModelKind::Mount,
                        model: PackedCharaModelId {
                            model_id: 1,
                            base_id,
                            variant_id,
                            chara_type: CharaModelType::Demihuman,
                        },
                    };
                    let mut filtered = SlotFilter {
                        inner: source.clone(),
                        allow_suffix: suffix,
                    };
                    if let Ok((data, skeleton)) =
                        assets::load_chara_model_with_skeleton_from_resource(&mut filtered, &req)
                    {
                        instances.push(self.renderer.context().create_model_with_skeleton(
                            &data,
                            opts.clone(),
                            skeleton.as_ref(),
                        ));
                    }
                }
            }
        }
        self.standalone = instances;
        Ok(())
    }

    pub fn clear_standalone(&mut self) {
        self.standalone.clear();
    }
}
