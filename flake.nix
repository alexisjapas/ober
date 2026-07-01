{
  description = "dj-mix — logiciel de mix DJ open-source (Rust + Bevy), POC Hercules DJControl Inpulse 200 MK2";

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

        # Toolchain stable pilotée par rust-toolchain.toml (source unique de vérité,
        # partagée avec les utilisateurs rustup). Reproductible via flake.lock.
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Bibliothèques système nécessaires à la compilation et au runtime :
        # cpal/midir (ALSA — PipeWire fonctionne via la couche ALSA, specs §3.2),
        # Bevy/wgpu/winit (Vulkan, Wayland + fallback X11, libxkbcommon, udev).
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
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            toolchain
            pkgs.pkg-config
          ];

          # Sous macOS, les frameworks système (CoreAudio, CoreMIDI, Metal…)
          # sont fournis automatiquement par le SDK du stdenv nixpkgs.
          buildInputs = runtimeLibs;

          packages = lib.optionals stdenv.isLinux (
            with pkgs;
            [
              # aseqdump : rétro-ingénierie MIDI du contrôleur (specs §5.3),
              # en complément de notre outil `midi-probe`.
              alsa-utils
            ]
          );

          # wgpu/winit chargent vulkan-loader, wayland et libxkbcommon
          # dynamiquement à l'exécution (dlopen) : elles doivent être visibles.
          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
        };
      }
    );
}
