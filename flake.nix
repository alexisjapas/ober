{
  description = "ober — open-source DJ mixing software (Rust + Bevy), Hercules DJControl Inpulse 200 MK2 POC";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        inherit (pkgs) lib stdenv;

        # Stable toolchain driven by rust-toolchain.toml (single source of truth,
        # shared with rustup users). Reproducible through flake.lock.
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # System libraries needed at build time and at runtime:
        # cpal/midir (ALSA — PipeWire works through its ALSA layer, specs §3.2),
        # Bevy/wgpu/winit (Vulkan, Wayland + X11 fallback, libxkbcommon, udev).
        runtimeLibs = lib.optionals stdenv.isLinux (
          with pkgs;
          [
            alsa-lib
            udev
            vulkan-loader
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
          ]
        );

        # `play [--release] [tracks…]`: runs the `ober` binary (crates/app),
        # debug by default, optimized with --release. Extra arguments are
        # forwarded to the app (preloaded track paths). Lives in the dev shell
        # (inherits LD_LIBRARY_PATH + toolchain); no versioned .sh script.
        play = pkgs.writeShellScriptBin "play" ''
          profile=()
          forward=()
          for arg in "$@"; do
            case "$arg" in
              --release) profile=(--release) ;;
              *) forward+=("$arg") ;;
            esac
          done
          exec cargo run "''${profile[@]}" -p app -- "''${forward[@]}"
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            toolchain
            pkgs.pkg-config
          ];

          # On macOS the system frameworks (CoreAudio, CoreMIDI, Metal…) are
          # provided automatically by the nixpkgs stdenv SDK.
          buildInputs = runtimeLibs;

          packages =
            [
              # Third-party license notices (about.toml) — same tool and version
              # family as the release CI, for local runs of `cargo about generate`.
              pkgs.cargo-about
              # `play [--release] [tracks…]`: run the app (defined in the `let` above).
              play
            ]
            ++ lib.optionals stdenv.isLinux (
              with pkgs;
              [
                # aseqdump: MIDI reverse-engineering of the controller (specs §5.3),
                # complementing our own `midi-probe` tool.
                alsa-utils
              ]
            );

          # wgpu/winit dlopen vulkan-loader, wayland and libxkbcommon at runtime:
          # they must be visible on the library path.
          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;

          shellHook = ''
            echo "ober dev shell — $(rustc --version)"
            echo "  play [--release] [tracks…]  # run the app, debug by default (optional preloaded tracks)"
          '';
        };
      }
    );
}
