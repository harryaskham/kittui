{
  description = "kittui — Rust-native kitty graphics renderer for TUIs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        workspaceManifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        workspaceVersion = workspaceManifest.workspace.package.version;

        commonArgs = {
          version = workspaceVersion;
          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          strictDeps = true;
          buildInputs = lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
        };

        kittui = pkgs.rustPlatform.buildRustPackage (
          commonArgs
          // {
            pname = "kittui";
            cargoBuildFlags = [
              "-p"
              "kittui-cli"
            ];
            cargoTestFlags = [
              "-p"
              "kittui-cli"
            ];
            meta = {
              description = "CLI for rendering kitty graphics from kittui scenes";
              homepage = "https://github.com/harryaskham/kittui";
              license = with lib.licenses; [
                mit
                asl20
              ];
              mainProgram = "kittui";
              platforms = lib.platforms.unix;
            };
          }
        );

        kittui-ffi = pkgs.rustPlatform.buildRustPackage (
          commonArgs
          // {
            pname = "kittui-ffi";
            cargoBuildFlags = [
              "-p"
              "kittui-ffi"
            ];
            cargoTestFlags = [
              "-p"
              "kittui-ffi"
            ];
            installPhase = ''
              runHook preInstall
              mkdir -p "$out/lib"
              find target -type f \
                \( -name 'libkittui_ffi.a' -o -name 'libkittui_ffi.so' -o -name 'libkittui_ffi.dylib' \) \
                -exec cp -v {} "$out/lib/" \;
              runHook postInstall
            '';
            meta = {
              description = "C ABI library for kittui";
              homepage = "https://github.com/harryaskham/kittui";
              license = with lib.licenses; [
                mit
                asl20
              ];
              platforms = lib.platforms.unix;
            };
          }
        );

        workspace-check = pkgs.rustPlatform.buildRustPackage (
          commonArgs
          // {
            pname = "kittui-workspace-check";
            cargoBuildFlags = [
              "--workspace"
              "--all-targets"
            ];
            cargoTestFlags = [ "--workspace" ];
            dontInstall = true;
          }
        );
      in
      {
        packages = {
          default = kittui;
          inherit kittui kittui-ffi;
        };

        apps = {
          default = self.apps.${system}.kittui;
          kittui = {
            type = "app";
            program = "${kittui}/bin/kittui";
            meta.description = "Run the kittui CLI";
          };
        };

        checks = {
          inherit kittui kittui-ffi workspace-check;
          default = workspace-check;
        };

        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              cargo
              cargo-nextest
              clippy
              just
              nixd
              nixfmt
              pkg-config
              rust-analyzer
              rustc
              rustfmt
            ]
            ++ lib.optionals pkgs.stdenv.isDarwin [ libiconv ]
            ++ lib.optionals pkgs.stdenv.isLinux [
              # Real Xvfb backend for kittui-wm + kittui-xvfb proof harness.
              xorg.xorgserver
              xorg.xvfb
              xorg.libxcb
              xorg.libX11
              xorg.libXtst
              xorg.libXext
              libxkbcommon
              xorg.xeyes
              xorg.xterm
              xorg.xclock
            ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };

        formatter = pkgs.nixfmt;
      }
    );
}
