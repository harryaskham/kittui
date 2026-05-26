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
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "mcp-cli-0.0.1" = "sha256-aEWGvQh5YklD8l8bylHGIakhYovabHmPHtbGPjXM/1w=";
              "updatable-cli-0.1.0" = "sha256-kwsURSIbPW4o1S+YGGPwxWG8td4uZz8UYx6RIYMr5Ek=";
            };
          };
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.libghostty-vt ] ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
        };

        # Platform-native default features so the installed package always
        # ships a working backend for kittwm (sck/quartz on macOS, xvfb on
        # Linux) without requiring rebuild flags. (bd-1bbee6)
        kittuiCliFeatures =
          if pkgs.stdenv.isDarwin then
            [ "sck" ]
          else if pkgs.stdenv.isLinux then
            [ "xvfb" ]
          else
            [ ];
        cargoFeatureFlags =
          if kittuiCliFeatures == [ ] then
            [ ]
          else
            [
              "--features"
              (lib.concatStringsSep "," kittuiCliFeatures)
            ];

        checkEnv = {
          preCheck = ''
            export KITTUI_CACHE_DIR="$TMPDIR/kittui-cache"
            export XDG_CACHE_HOME="$TMPDIR/xdg-cache"
            mkdir -p "$KITTUI_CACHE_DIR" "$XDG_CACHE_HOME"
          ''
          + lib.optionalString pkgs.stdenv.isDarwin ''
            export KITTWM_PTY_SHELL=${pkgs.bash}/bin/bash
          '';
        }
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          KITTWM_PTY_SHELL = "${pkgs.bash}/bin/bash";
        };

        kittui = pkgs.rustPlatform.buildRustPackage (
          commonArgs
          // checkEnv
          // {
            pname = "kittui";
            cargoBuildFlags = [
              "-p"
              "kittui-cli"
            ]
            ++ cargoFeatureFlags;
            cargoTestFlags = [
              "-p"
              "kittui-cli"
            ]
            ++ cargoFeatureFlags;
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
          // checkEnv
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
          kittwm = kittui;
          inherit kittui kittui-ffi;
        };

        apps = {
          default = self.apps.${system}.kittui;
          kittui = {
            type = "app";
            program = "${kittui}/bin/kittui";
            meta.description = "Run the kittui CLI";
          };
          kittwm = {
            type = "app";
            program = "${kittui}/bin/kittwm";
            meta.description = "Run the kittwm terminal-native window manager";
          };
          kittwm-browser = {
            type = "app";
            program = "${kittui}/bin/kittwm-browser";
            meta.description = "Run the kittwm browser surface helper";
          };
          kittwm-terminal = {
            type = "app";
            program = "${kittui}/bin/kittwm-terminal";
            meta.description = "Run the kittwm terminal surface client";
          };
          kittwm-launch = {
            type = "app";
            program = "${kittui}/bin/kittwm-launch";
            meta.description = "Run the kittwm SDK app/surface launcher";
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
              libghostty-vt
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
              xterm
              xclock
            ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };

        formatter = pkgs.nixfmt;
      }
    );
}
