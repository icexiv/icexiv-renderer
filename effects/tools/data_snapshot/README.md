# Offline importer source

The viewer pins `icexiv-data@919ffe3c9b0b4f64337a2bf12ebc94c5bffa6fa6` for its small resource readers. That revision predates the action TMB API. For standalone authoring, `tmb.rs` is a byte-for-byte copy of the inspected `icexiv-data/src/tmb.rs` (SHA-256 `ba52ace117f73f35bef8663274f3c1f43913b6c721de0755dc6a6a7b3bc3c4d2`); `binary.rs` and `havok.rs` supply its private checked slice and budget helpers. No game bytes are included. The runtime only consumes decoded commands.

`cargo run --manifest-path effects/tools/Cargo.toml --locked -- …` uses this snapshot. To compare with a working data checkout, run `import-action.py DATA_CHECKOUT …` with the same arguments. Compare both the manifest and file digests. When a published data revision contains these interfaces, replace the snapshot with that dependency. The sheet column checks in `pack_action.rs` correspond to `Row::action_timeline_id` and `Row::action_timeline_key` in the same data worktree.
