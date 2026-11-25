{
  description = "IceXIV native and WebGPU equipment renderer";

  inputs = {
    notist.url = "github:AzurIce/notist";
    nixpkgs.follows = "notist/nixpkgs";
    crane.follows = "notist/crane";
    rust-overlay.follows = "notist/rust-overlay";
    flake-utils.follows = "notist/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      crane,
      rust-overlay,
      flake-utils,
      notist,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rust;
        cli = pkgs.buildWasmBindgenCli rec {
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            version = "0.2.129";
            hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
          };
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            inherit (src) pname version;
            hash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";
          };
        };
        source =
          paths:
          pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions paths;
          };
        common = {
          pname = "icexiv-renderer";
          version = (fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = source [
            ./Cargo.toml
            ./Cargo.lock
            ./src
            ./examples
            ./web
            ./package.json
            ./README.md
          ];
          strictDeps = true;
          CARGO_BUILD_JOBS = "4";
          cargoExtraArgs = "--locked --lib";
        };
        build =
          args:
          craneLib.mkCargoDerivation (
            common
            // args
            // {
              cargoArtifacts = craneLib.buildDepsOnly (
                common
                // pkgs.lib.optionalAttrs (args ? CARGO_BUILD_TARGET) {
                  inherit (args) CARGO_BUILD_TARGET;
                }
              );
              buildPhaseCargoCommand = "cargo build --locked --release --lib -j4";
              doCheck = false;
            }
          );
        native = build {
          installPhaseCommand = "install -Dm644 target/release/libicexiv_renderer.rlib $out/lib/libicexiv_renderer.rlib";
        };
        action-import = craneLib.buildPackage {
          pname = "icexiv-action";
          version = common.version;
          src = pkgs.lib.fileset.toSource {
            root = ./effects/tools;
            fileset = pkgs.lib.fileset.unions [
              ./effects/tools/Cargo.toml
              ./effects/tools/Cargo.lock
              ./effects/tools/import_action.rs
              ./effects/tools/pack_action.rs
              ./effects/tools/data_snapshot
            ];
          };
          CARGO_BUILD_JOBS = "3";
          doCheck = false;
        };
        effectSrc = pkgs.lib.fileset.toSource {
          root = ./effects;
          fileset = pkgs.lib.fileset.unions [ ./effects/Cargo.toml ./effects/Cargo.lock ./effects/src ./effects/scene ];
        };
        effectSdk = craneLib.mkCargoDerivation {
          pname = "icexiv-weapon-effects-web";
          version = common.version;
          src = effectSrc;
          cargoVendorDir = craneLib.vendorCargoDeps {
            src = effectSrc;
            # The companion playback ABI uses an archived revision outside data's main ref.
            overrideVendorGitCheckout = packages: drv:
              if pkgs.lib.any (p: p.name == "icexiv-data") packages then drv.overrideAttrs (_: {
                src = pkgs.fetchFromGitHub {
                  owner = "icexiv"; repo = "icexiv-data";
                  rev = "7fa1b24b9cac1b9892e8f6cdf290eb857fc9c08d";
                  hash = "sha256-CiwelWkbT9V8QV6ZRCuOLbqXRIYd3fjgl1G52S+LevM=";
                };
              }) else drv;
          };
          cargoArtifacts = null;
          doInstallCargoArtifacts = false;
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          nativeBuildInputs = [ cli ];
          buildPhaseCargoCommand = "cargo build --locked --release --lib -j4";
          installPhaseCommand = ''
            wasm-bindgen --target web --out-dir "$out" target/wasm32-unknown-unknown/release/icexiv_effects.wasm
          '';
        };
        sdk = build {
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          nativeBuildInputs = [ cli ];
          installPhaseCommand = ''
            mkdir -p $out
            wasm-bindgen --target web --out-dir $out/pkg target/wasm32-unknown-unknown/release/icexiv_renderer.wasm
            cp package.json README.md $out/
            cp web/component.js web/component.d.ts $out/pkg/
            mkdir -p $out/pkg/effects
            cp ${effectSdk}/* web/effect-worker.js $out/pkg/effects/
          '';
        };
        site =
          pkgs.runCommand "icexiv-renderer-docs"
            { nativeBuildInputs = [ notist.packages.${system}.default ]; }
            ''
              cp -r ${
                source [
                  ./Notist.toml
                  ./docs
                ]
              }/. .
              notist build
              cp -r notist-output/icexiv-renderer $out
            '';
      in
      {
        packages = {
          default = native;
          inherit native sdk site action-import;
        };
        checks = { inherit native sdk site action-import; };
        formatter = pkgs.nixfmt;
        devShells.default = craneLib.devShell {
          packages = [
            cli
            pkgs.nodejs
            pkgs.nixfmt
            notist.packages.${system}.default
          ];
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.isLinux (
            pkgs.lib.makeLibraryPath [ pkgs.vulkan-loader ]
          );
        };
      }
    );
}
