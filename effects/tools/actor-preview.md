# Dressed actor and held weapon

Export a character request and its actual logical resource closure from an installed client. Use the same `objects/` directory as the weapon/action export:

```sh
cargo run --release --locked --manifest-path effects/Cargo.toml -j2 --example export_actor -- \
  /path/to/game objects effects/examples/c0101-actor.json > actor.json
```

`await canvas.setActorBundle(actorManifest, read)` loads the 26 customize bytes, equipped pieces, base SKLB, and selected PAP record. `read(logicalPath, sha256)` returns the verified `Uint8Array`. With the actor attached, `canvas.playActionBundle(action, preview, read, {target:"actor"})` uses its unscaled SKLB rest locals in C010 gaps and applies body scaling once. To show an actor in `examples/action.html`, add `&actor=/actor.json` to the demo URL.

`EquipmentCanvas.setActor` adds a skinned actor to an open weapon scene. `clearActor()` returns to the weapon-only view. `entries` is an array of `[logicalPath, Uint8Array]` pairs for the dressed character loader: body, equipment, materials, textures, base and required partial skeletons. `customize` is the game's 26-byte ActorCustomize. Packed equipment IDs are decimal strings.

```js
await canvas.openEffect(weapon, read);
await canvas.setActor(entries, customize, baseSklb, idlePap, {
  record: 1, bone: "n_buki_r", loop: true,
  equipment: [{itemId: 7540, modelMain: "65677", equipSlotCategory: 4}],
});
```

The optional column-major `grip` maps weapon coordinates into hand-local space; its default is identity. One optional additive layer uses `overlay: {pap, record, weight}`. Actor and weapon have separate joint buffers; the body's scaled hand world frame drives the weapon and its resident effect bindings at each simulation tick. The camera is fitted to the scene's rest bounds, not refitted to the moving hand each frame.

**Partial face and hair rigs:** Each `actorPartialRigs()` entry is a logical SKLB path and owns a separate joint namespace. To move a single joint, read `{names, rest} = await canvas.actorPartialSkeleton(path)`, edit its parent-local T3/Q4/S3 row, and pass the full array to `setActorPartialPose(path, locals)`. The base C010 palette and both weapon attachment frames stay independent. `clearActorPartial(path)` restores just that rig.

```js
const face = (await canvas.actorPartialRigs()).find(p => p.endsWith("/skl_c0101f0002.sklb"));
// Logical PAP: chara/human/c0101/animation/f0002/resident/face.pap
await canvas.setActorPartialClip(face, facePapBytes, "cfxb_blink1", true);
```

The named PAP's binding must identify the selected rig and root (`c0101f0002_0:mdl:j_kao` in this example); normal and additive spline records use explicit track maps. A partial at rest inherits its base anchor's skin matrix. Partial clips start at zero on installation and follow the actor simulation clock. No simulated hair physics or implicit cross-rig retargeting is applied. Native equivalents are `actor_partial_rigs`, `actor_partial_skeleton`, `set_actor_partial_locals`, `set_actor_partial_clip`, and `clear_actor_partial`. The optional WebGPU fixture `?partial=1` exercises the real named blink.

**Native:** use `ActorAssets::load` with a `DressedCharacterLoadRequest`, then `EffectPlayer::set_actor`. `EffectPlayer::new_held_weapon` accepts an ordinary rigged weapon with no resident effect. Use `frame_with_host` to call `set_actor_locals` with unscaled parent-local T3/Q4/S3 in `actor_skeleton().bone_names` order, or `apply_actor_pose` with an already scaled `SkeletonPose`. Never apply character scaling twice.

**Real-data check:** from the repository root, inside the development shell:

```sh
XIV_GAME_DIR=/path/to/game ACTOR_FIXTURE_OUT=/path/to/private/fixture \
  cargo test --locked --manifest-path effects/Cargo.toml -j2 --test actor -- \
  --test-threads=2 --ignored
```

The test uses a c0101 actor, the Spring Dress, `cbnm_id0`, and the Saw of the Stars. At a fixed camera it moves the weapon out of frame to isolate its visible pixels, samples the body at two times, and compares AVFX on/off. It also checks return to standalone rendering. `ACTOR_FIXTURE_OUT` is optional; when present it writes a local screenshot and logical file bundles. To check WebGPU, place the CAS export (`actor.json`, `objects/`), `actor-browser.html`, and the built `pkg/` directory (as `sdk/`) alongside the generated `weapon.json`. Serve the fixture over localhost.
