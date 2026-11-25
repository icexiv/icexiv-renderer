//! Numeric adapter for a *successful* action-listener BPTP=2 return. The
//! selector is a Document occurrence / Linear binder / target-list ordinal.
use super::*;
// Private lookup key in the isolated per-occurrence numeric target list.
// This is not a game ElementId, and never appears in host binding snapshots.
pub(super) const QUERY_SLOT: i32 = 1;

#[derive(Deserialize)]
pub struct DirectionalGoalFrame {
    pub occurrence: usize,
    pub binder: usize,
    pub ordinal: usize,
    /// Final listener query matrix, after target virtual +0x1a8 and the
    /// listener transform. Not a bone, ElementId or target object's origin.
    #[serde(flatten)]
    pub result: DirectionalQuery,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum DirectionalQuery {
    Matrix {
        basis: [[f32; 3]; 3],
        position: [f32; 3],
    },
    Missing {
        missing: bool,
    },
}

// Keep the object input private to this occurrence. In this shape no other
// binder can read the target object's kind-0 source, and the caster still has
// its own object and ElementId providers.
pub(super) fn eligible(file: &AvfxFile) -> bool {
    let [b] = file.binders.as_slice() else {
        return false;
    };
    let (Some(s), Some(g)) = (&b.properties_start, &b.properties_goal) else {
        return false;
    };
    b.binder_type == 1
        && b.properties_1.is_none()
        && b.properties_2.is_none()
        && s.bind_point_type == 0
        && s.bind_target_point_type == 3
        && g.bind_point_type == 1
        && g.bind_target_point_type == 2
        && g.bind_point_id == 0
        && matches!(g.binder_name.as_str(), "" | "null")
        && g.bct == 0
        && !g.ring_enabled
}

impl DirectionalGoalFrame {
    pub(super) fn resolved(&self) -> Result<Arc<VfxBinderTargetObjectSnapshot>> {
        ensure!(
            self.binder == 0 && self.ordinal == 0,
            "only Linear binder 0 goal / target-list ordinal 0 is supported"
        );
        let targets = match self.result {
            DirectionalQuery::Matrix { basis, position } => {
                ensure!(
                    basis
                        .into_iter()
                        .flatten()
                        .chain(position)
                        .all(f32::is_finite),
                    "nonfinite directional goal matrix"
                );
                vec![VfxBinderTargetSnapshot {
                    id: QUERY_SLOT as u32,
                    matrix: VfxBinderMatrix { basis, position },
                }]
            }
            DirectionalQuery::Missing { missing } => {
                ensure!(
                    missing,
                    "an unsuccessful directional query must specify missing:true"
                );
                vec![]
            }
        };
        Ok(Arc::new(VfxBinderTargetObjectSnapshot {
            object: None,
            targets,
            vfx_scale: 1.,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_original_listener_return_and_rejects_unrepresentable_frames() {
        // Lifted original 14033a240 with finalizer 14038da70: provider Y+90,
        // T=(1,2,3), listener diag=(2,3,4), T=(5,6,7).
        let basis = [[0., 0., -2.], [0., 3., 0.], [4., 0., 0.]];
        let position = [8., 8., -2.];
        let mut f = DirectionalGoalFrame {
            occurrence: 0,
            binder: 0,
            ordinal: 0,
            result: DirectionalQuery::Matrix { basis, position },
        };
        let target = f.resolved().unwrap();
        let m = target.targets[0].matrix;
        assert_eq!(m.position, position);
        for (a, b) in m
            .basis
            .into_iter()
            .flatten()
            .zip(basis.into_iter().flatten())
        {
            assert!((a - b).abs() < 2e-6);
        }
        assert!(target.object.is_none());
        let mut shear = basis;
        shear[1][2] = 1.;
        f.result = DirectionalQuery::Matrix {
            basis: shear,
            position,
        };
        assert_eq!(f.resolved().unwrap().targets[0].matrix.basis, shear);
        shear[1][2] = f32::NAN;
        f.result = DirectionalQuery::Matrix {
            basis: shear,
            position,
        };
        assert!(f.resolved().is_err());
        f.result = DirectionalQuery::Missing { missing: true };
        assert!(f.resolved().unwrap().targets.is_empty());
        f.ordinal = 1;
        assert!(f.resolved().is_err());
    }
}
