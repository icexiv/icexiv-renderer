//! Bone-attached 3D props (e.g. C198 emote items: teacups, brooms, books, food).
use super::{EffectAssets, EffectPlayer, pose};
use anyhow::{Context, Result};
use assets::{ModelSkeleton, WeaponModelData};
use scene::{ModelInstance, ModelRenderContext, PreparedModelOptions};

pub struct PropAssets {
    pub model: WeaponModelData,
    pub options: PreparedModelOptions,
    pub skeleton: Option<ModelSkeleton>,
}

impl PropAssets {
    pub fn load<R: physis::resource::Resource>(source: &mut R, main: u64) -> Result<Self> {
        let (model, options) = EffectAssets::owner(source, main)?;
        let sk_path = assets::weapon_skeleton_path(model.model_main.model_id);
        let skeleton = match source.read(&sk_path) {
            Ok(bytes) => Some(assets::load_skeleton_from_sklb_bytes(&bytes)?),
            Err(physis::Error::FileNotFound { .. }) => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            model,
            options,
            skeleton,
        })
    }
}

pub(super) struct Prop {
    pub(super) id: String,
    pub(super) bone: String,
    pub(super) visible: bool,
    pub(super) mesh: ModelInstance,
    bind: Option<pose::OwnerPose>,
    rest: Vec<f32>,
    grip: [f32; 16],
    last_hand: Option<[f32; 16]>,
}

impl Prop {
    pub(super) fn new(
        ctx: &ModelRenderContext,
        id: String,
        data: PropAssets,
        bone: String,
        grip: [f32; 16],
    ) -> Self {
        let mesh = ctx.create_model_with_skeleton(
            &data.model,
            data.options.with_component_preview_layout(false),
            data.skeleton.as_ref(),
        );
        let (bind, rest) = match data.skeleton {
            Some(sk) => {
                let owner = pose::OwnerPose::from_bind(sk);
                let rest = owner.rest_locals();
                (Some(owner), rest)
            }
            None => (None, Vec::new()),
        };
        Self {
            id,
            bone,
            visible: true,
            mesh,
            bind,
            rest,
            grip,
            last_hand: None,
        }
    }

    pub(super) fn pose_to_hand(&mut self, ctx: &ModelRenderContext, hand: [f32; 16]) -> Result<()> {
        if self.last_hand == Some(hand) {
            return Ok(());
        }
        let Some(bind) = &mut self.bind else {
            return Ok(());
        };
        if self.mesh.joint_names().is_empty() {
            return Ok(());
        }
        let trs = bind.gripped_locals(&hand, &self.grip, &self.rest)?;
        let mut empty = Default::default();
        let (matrices, _) = bind.resolve(&trs, self.mesh.joint_names(), &mut empty, None)?;
        self.mesh.update_joint_matrices(ctx, &matrices);
        self.last_hand = Some(hand);
        Ok(())
    }
}

impl EffectPlayer {
    pub fn add_prop(
        &mut self,
        id: String,
        data: PropAssets,
        bone: String,
        grip: [f32; 16],
    ) -> Result<()> {
        self.props.retain(|p| p.id != id);
        let mut prop = Prop::new(self.renderer.context(), id, data, bone, grip);
        if let Some(actor) = &self.actor
            && let Some(hand) = actor.bone_world(&prop.bone)
        {
            prop.pose_to_hand(self.renderer.context(), hand)?;
        }
        self.props.push(prop);
        Ok(())
    }

    pub fn set_prop_visible(&mut self, id: &str, visible: bool) -> Result<()> {
        let prop = self
            .props
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| format!("unknown prop: {id}"))?;
        prop.visible = visible;
        Ok(())
    }

    pub fn clear_props(&mut self) {
        self.props.clear();
    }

    pub(super) fn update_props(&mut self) -> Result<()> {
        let Some(actor) = &self.actor else {
            return Ok(());
        };
        for prop in &mut self.props {
            if let Some(hand) = actor.bone_world(&prop.bone) {
                prop.pose_to_hand(self.renderer.context(), hand)?;
            }
        }
        Ok(())
    }
}
