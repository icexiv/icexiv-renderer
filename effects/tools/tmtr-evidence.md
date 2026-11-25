# TMTR condition evaluation

The checked Win64 `ffxiv_dx11.exe` has SHA-256 `52042b60631cf2c48851b3aaee36e9ec4fedd6d7f2c68f0e9a8c3befb0af9f98`. Addresses below are preferred-image virtual addresses.

* **Track gate:** `0x141c2d890` tests the track payload's `+0x0c` relative offset. With a condition it calls `0x1418d21d0` and stores the resulting boolean at track-instance `+0x32`. That helper calls the expression builder, then `0x1418d3230`, and converts nonzero to true.
* **Tokens and inputs:** `0x1418d1ef0` walks the condition block using its offset, count and 12-byte records. It decodes the string table at `0x1428d73a0`; opcodes 1/2 repeat literal parentheses, 17 formats signed `%d`, 18 formats promoted f32 with `%f`, and 19 first asks the optional provider at virtual slot `+8`. If unhandled, selector bits 28–31 choose an integer bank and bits 0–27 index it. The SDK accepts resolved host values, with per-TMB/per-actor values taking precedence over shared ones. It does not assume a particular actor state, or use zero for missing inputs.
* **Evaluation:** `0x1418d3520` recursively processes precedence tables (`0x1428d7660`); the operations are signed integer arithmetic/comparisons, and `and`/`or` are bitwise. The lexer at `0x1418d3bc0` consumes the full numeric token but copies at most nine bytes; `0x141c0e580` handles `true`, `false`, or a base-10 integer prefix. The text builder's destination spans manager `+0x200..+0x380`. The SDK rejects over-budget text, malformed expressions, invalid division and unsupported function tokens rather than reproducing unchecked or exceptional native behavior.

REA Rizin evidence: `ev_f9a609f5615e80b5ff8b39f03f0eda9abf437db057fac589f1372630df016850` (builder, parser entry, tables); `ev_11d24f76a8be207d4cb8fc27c0e5c054bee1d391c338f6fe83e22c7e406a4e7a` (track call, variable banks, comparison/bitwise and leaf operations).

```sh
rea execute-rizin-command "$EXE" 'pD 572 @ 0x1418d1ef0; pD 46 @ 0x1418d2140; px 704 @ 0x1428d73a0; px 64 @ 0x1428d7660; pD 116 @ 0x1418d3520' --json
rea execute-rizin-command "$EXE" 'pD 190 @ 0x141c2d8b0; pD 42 @ 0x1418d21d0; pD 57 @ 0x1418d1e60; pd 4 @ 0x1418d2d00; pd 3 @ 0x1418d2c30; pd 3 @ 0x1418d3060; pD 98 @ 0x141c0e580' --json
```

## Executable comparison

`verify-tmtr-native.py` reads the original TMB records, loads the identified PE into Unicorn, supplies integer variable-bank values and calls the original x86 builder and evaluator. Only CRT formatting, short string/memory operations, ASCII case conversion, decimal conversion and the stack cookie check are shimmed. This is execution of the evaluator with controlled inputs, not an observation of a live actor.

| Resource | Checked behavior |
|---|---|
| `battle/magic_cnj_end.tmb`, track 3 | `(((((V==35))))==false)`, where `V` is `0x10000029` |
| Same file, track 4 | `(V==35)`; for `V=35` only track 4 passes, for `V=42` only track 3 passes |
| `ability/swd_knight/abl010.tmb`, track 3 | `0x10000013 == 1` |
| `ability/pvp_common/abl027_ded.tmb`, track 5 | 39 parenthesized equalities joined by `or`; values 1 and 35 pass, 0 and 42 fail |
| Same file, track 7 | `(V==40)` |

Across the 1,500-key ActionTimeline lookup sample, 26 extracted TMB files contained 29 nonempty condition blocks. Adding Stone's child TMB gave 31 blocks and five distinct scripts. The SDK matched all 124 native branch results for values `0, 1, 35, 42`, plus four controlled probes covering bitwise `and`, arithmetic precedence, `%f`/integer conversion and the numeric token width. This does not establish coverage of every game TMB or math function.

```sh
python effects/tools/verify-tmtr-native.py "$EXE" \
  "$EXTRACTED/chara/action/battle/magic_cnj_end.tmb" \
  "$EXTRACTED/chara/action/ability/pvp_common/abl027_ded.tmb" \
  "$EXTRACTED/chara/action/ability/swd_knight/abl010.tmb" --operators > rows.jsonl
TMTR_NATIVE_ROWS="$PWD/rows.jsonl" cargo test --locked --manifest-path effects/Cargo.toml \
  -j4 --lib native_condition_rows -- --ignored --test-threads=1
```
