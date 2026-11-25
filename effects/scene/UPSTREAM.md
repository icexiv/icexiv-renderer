# Scene source

Base: [`AzurIce/xiv-companion` at `1a7950ac5425c5096d3eda655184ae6a85fe8b43`](https://github.com/AzurIce/xiv-companion/tree/1a7950ac5425c5096d3eda655184ae6a85fe8b43/crates/xiv-companion-render), directory `crates/xiv-companion-render`, Git tree `7a5aa3d5c38607b61470197eadd51f115e38ddcc`. `build.rs` compiles the included WESL. No game executable, DXBC or extracted shader package is part of this copy.

The local manifest expands the parent workspace's package fields and `xiv-companion-data` dependency. Keep its data revision aligned with `effects/Cargo.toml`. Local rendering changes include [Apricot forward particle tone](../../docs/particle-tone.md); inspect the actual delta before upgrading:

```sh
# Run in the renderer checkout. XIV_COMPANION is a local upstream Git checkout.
base=1a7950ac5425c5096d3eda655184ae6a85fe8b43
scratch=$(mktemp -d)
git -C "$XIV_COMPANION" archive "$base":crates/xiv-companion-render | tar -xC "$scratch"
git diff --no-index "$scratch" effects/scene   # exit 1 = differences
```

For an update, review `git -C "$XIV_COMPANION" diff "$base" "$new" -- crates/xiv-companion-render` and merge those changes into the local copy, retaining local APIs, shader changes and manifest adjustments. Update this base commit/tree and `effects/Cargo.lock`. Verify native and wasm consumers with `cargo check --manifest-path effects/Cargo.toml` (also `--target wasm32-unknown-unknown`); run the pixel regression in `docs/particle-tone.md`. Upstream's fixture-backed unit tests refer to its sibling data checkout; this directory is the render crate, not a mirror of that workspace.
