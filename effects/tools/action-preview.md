# Import and play an action

`icexiv-action` is a standalone native tool (`cargo build --locked --release --manifest-path effects/tools/Cargo.toml`, or `nix build .#action-import`). It uses the pinned data readers and the [TMB authoring snapshot](data_snapshot/README.md). For source parity checks `import-action.py` can build the same adapter against an explicit `icexiv-data` checkout exposing the `tmb` API. The shipped native and WASM players consume its JSON.

Keep extracted files under their logical game paths. For Stone (Action 119, ActionTimeline 448):

```sh
python effects/tools/import-action.py "$DATA_CHECKOUT" "$EXTRACTED" \
  chara/action/magic/cnj_white/stone.tmb > stone.json
cargo run --manifest-path effects/Cargo.toml --example timeline --release -- \
  "$EXTRACTED" stone.json effects/examples/stone-preview.json 70
python effects/tools/verify-stone.py "$EXTRACTED" stone.json effects/target/release/examples/timeline
```

In the browser, call `await element.openWeapon(ownerManifest, read)` then `await element.playAction(events, preview, entries)`. `entries` contains `[logicalPath, Uint8Array]` pairs for the referenced AVFX and their textures. `playAction` returns load diagnostics. Later AVFX fallbacks arrive through `renderwarning`. `replayEffect()` restarts the action and `stopAction()` removes it.

Each JSON list member is **one C012 occurrence**, with its chain of reachable C002 calls. `(tmb, actor, track, command)` identifies each source command; occurrence order distinguishes repeated calls. The importer preserves conditions and both binding selectors, including ID `-1`. Only asset decoding is shared.

The example clock is a **preview policy**: 30 raw TMB units/s, additive actor/track/command starts in each child, and half-open command windows clipped to parent windows and time zero. Birth starts at the clipped time. The example's object and ElementId transforms are synthetic. Real callers supply resolved world-space transforms for each `(origin, kind, id)`: caster/target = `1/2`, character/weapon/offhand = `0/1/2`. The first supplies the Document and caster provider; the second supplies the target-list object. An AVFX binder's ElementId is looked up *on* that object, not in the C012 ID namespace.

At load, C002 ancestors, C010 windows and C012 occurrences use the same TMTR condition gate. A nonempty `enabled_tracks` is an authoritative list for conditional tracks; the Stone example uses this to show both children. To execute the conditions instead, clear the list and supply the integer snapshot: `preview.enabled_tracks=[]; preview.condition_inputs={variables:{"268435497":35}}`. This selects Stone child track 4; value 42 selects track 3. Optional `condition_inputs.actors[tmbPath][actorId]` maps packed selectors to actor-specific values. Missing variables or unsupported operations diagnose and skip the occurrence. With neither an explicit selection nor inputs, conditional tracks remain inactive. [Executable evidence and reproduction](tmtr-evidence.md).

For an identity host, C012 TRS supplies the Document matrix automatically: signed scale, rotations about X then Y then Z (radians), then translation. For nondefault TRS combined with a nonidentity external host, supply a resolved `preview.command_frames` entry `{occurrence, basis, position, scale}`. The columns and translation give the final Document matrix; `scale` is its independent scale getter. Neither path reinterprets the endpoint selectors or adds a post-sampling offset to bound particles. Each occurrence's `color` is linear RGBA: command RGB multiplies AVFX revised RGB, and command alpha is independent. Both apply at fragment time. Controller masks `1` (external lifetime), `2` (eligibility bypass), and `4` (state-32 stop) compose independently. All flag words `0..7` are admitted subject to the resolved scene inputs. Unknown bits, nonpositive durations and nonzero ability delays remain diagnosed and skipped. See the [controller API](../../docs/action.md#c012-控制器). See [C012 evidence and real-resource checks](c012-evidence.md), including why Stone's Linear Item binder falls back.

`element.setActionElements(selector, elements)` replaces the first endpoint's world-space ElementIds. `element.setActionTarget(binding)` queues a complete second endpoint using the same `{target, position, quaternion?, scale?, elements?}` format as `preview.bindings`. Omitting `elements` supplies an empty list. Each ElementId has `{id, basis, position}` in world coordinates. The native equivalents are `ActionPlayer::set_caster_elements`, `ActionPlayer::set_target_binding` and `EffectPlayer::set_action_target`.

New C012 occurrences use the latest target at birth. Existing Documents retain target ordinal zero and install the latest object and ElementIds together on the next **positive** local-time input; paused redraws do not consume queued target updates. When caster or camera changes at the same step, the preview refreshes those inputs at the old age, then advances once with the new target. Identical reads do not issue another simulation input. Without a camera, a continuous-fallback Document can only install one changed provider on a positive input; a concurrent caster update stays queued and emits a diagnostic. This is host scheduling, not a claim about the client's input order. AVFX controls when each binder queries its providers and how existing particles follow them.

## Named C010 clips

```sh
python effects/tools/import-action.py "$DATA_CHECKOUT" "$EXTRACTED" \
  chara/action/normal/idle.tmb --clips > idle-motion.json
```

`--clips` emits `{effects, clips}`. Each reachable C010 has a `name` and the same raw `chain` metadata as C012 (including actor, track, command IDs, signed start, duration, conditions, and parent C002 calls). Without the flag, the importer emits the original C012 array. C009 is not promoted to a C010.

```js
await element.playAction(manifest, preview, avfxAndTextureEntries, {
  sklb: actorSklbBytes,
  paps: {cbnm_id0: idlePapBytes}, // C010 record name -> explicit PAP bytes
  target: "hand", bone: "n_buki_r", grip: weaponToHandMatrix,
  gapHand: suppliedHandMatrix,
  // Optional, with exactly one mounted weapon: stream its posed ElementIds.
  // weaponBinding: {origin: 1, kind: 1, id: -1}
});
```

Instead of just the hand, set `target: "actor"` for an attached dressed actor. Supply `sklb`, `paps` and `gapLocals` (packed local T3/Q4/S3 rows in that actor's skeleton order). Its posed hand also drives the gripped weapon. Clocks, conditions and clipping use the same preview policy as C012. Within a chosen window, sample seconds since its clipped start and hold the PAP's last frame if shorter. Outside it, use the explicit gap pose. Only unambiguous, normal named PAP records are accepted. The caller must select one nonoverlapping pose layer; the worker does not guess actor ownership, PAP file paths, blend weights, transitions or loop flags. Replay rewinds the action-local clock.

For an actor, omitting `gapLocals` (or `gap: "rest"`) selects its rest pose. For a hand, omitting `gapHand` selects the matching SKLB's rest bone matrix. `crossfade_clips: true` in the preview permits an ordered two-clip transition over their overlap, including independent local times for repeated records. It does not infer a client transition curve.

For a browser check, serve `verify-c010.html` alongside a built `sdk/` directory, an exported `manifest.json` and `objects/` for a weapon, `idle-motion.json`, and the matching `actor.sklb`, `idle.pap` and `move_a.pap` (c0101). The page exercises the real worker, named record lookup, zero-time birth, gaps, switching between two PAPs, mismatch rejection and replay. Only the first schedule is the original idle TMB; the two-clip transition schedule is a controlled fixture.

Add `?actor` to that page with `actor/` containing the dressed fixture emitted by `effects/tests/actor.rs` to run the same switch on the character and gripped weapon.

To verify the live ElementId path on real Stone data:

```sh
python effects/tools/import-action.py "$DATA_CHECKOUT" "$EXTRACTED" \
  chara/action/magic/cnj_white/stone.tmb --clips > stone-motion.json
ACTION_ROOT="$EXTRACTED" STONE_MANIFEST=stone-motion.json \
  cargo test --manifest-path effects/Cargo.toml --release -j4 --test timeline \
  stone_live_element_and_camera -- --ignored --test-threads=1
```

For the main Stone occurrence, set `preview.directional_goals` to an array of
`{occurrence, binder: 0, ordinal: 0, basis, position}` records **before birth**.
The occurrence index is 2 in the full Stone import. `basis` and `position` are
the final **directional listener query** return, not the target's generic
object TRS or an ElementId. Supply caster ElementId 16 separately. Call
`await element.setDirectionalGoal(frame)` to refresh the result on subsequent
positive inputs. Use `{occurrence,binder:0,ordinal:0,missing:true}` when the
provider returned false, including at birth. Affine columns can include shear.
Do not invent the result from two positions:
see [the native inputs and tested reduction](c012-evidence.md#stone-the-directional-goal-query).
