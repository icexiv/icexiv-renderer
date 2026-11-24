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
        sdk = build {
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          nativeBuildInputs = [ cli ];
          installPhaseCommand = ''
            mkdir -p $out
            wasm-bindgen --target web --out-dir $out/pkg target/wasm32-unknown-unknown/release/icexiv_renderer.wasm
            cp package.json README.md $out/
            cp web/component.js $out/pkg/
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
          inherit native sdk site;
        };
        checks = { inherit native sdk site; };
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
