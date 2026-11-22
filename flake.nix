{
  description = "mu — a minimal coding agent";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.treefmt-nix = {
    url = "github:numtide/treefmt-nix";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      treefmt-nix,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      each = nixpkgs.lib.genAttrs systems;
      treefmt = each (system: treefmt-nix.lib.evalModule nixpkgs.legacyPackages.${system} ./treefmt.nix);
    in
    {
      formatter = each (system: treefmt.${system}.config.build.wrapper);
      checks = each (system: {
        formatting = treefmt.${system}.config.build.check self;
      });

      packages = each (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "mu";
            version = "0.1.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./crates
                ./Cargo.toml
                ./Cargo.lock
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            stripAllList = [ "bin" ];
            postInstall = ''
              wrapProgram $out/bin/mu --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  pkgs.bash
                  pkgs.curl
                  pkgs.ripgrep
                ]
              }
            '';
          };
        }
      );
      devShells = each (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rust-analyzer
              clippy
              bash
              curl
              ripgrep
              treefmt.${system}.config.build.wrapper
            ];
          };
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          android =
            let
              pkgs = import nixpkgs {
                inherit system;
                overlays = [ rust-overlay.overlays.default ];
                config = {
                  allowUnfree = true;
                  android_sdk.accept_license = true;
                };
              };
              buildTools = "37.0.0";
              ndk = "30.0.16248370";
              android = pkgs.androidenv.composeAndroidPackages {
                platformVersions = [ "37.0" ];
                buildToolsVersions = [ buildTools ];
                includeNDK = true;
                ndkVersions = [ ndk ];
                includeEmulator = false;
                includeSystemImages = false;
              };
              sdk = "${android.androidsdk}/libexec/android-sdk";
              rust = pkgs.rust-bin.stable.latest.default.override {
                extensions = [
                  "rust-src"
                  "rust-analyzer"
                ];
                targets = [
                  "aarch64-linux-android"
                  "x86_64-linux-android"
                ];
              };
              jdk = pkgs.jdk21;
            in
            pkgs.mkShell {
              packages = [
                rust
                pkgs.cargo-ndk
                android.androidsdk
                jdk
                pkgs.gradle_9
                treefmt.${system}.config.build.wrapper
              ];
              ANDROID_HOME = sdk;
              ANDROID_SDK_ROOT = sdk;
              ANDROID_NDK_HOME = "${sdk}/ndk/${ndk}";
              JAVA_HOME = jdk.home;
              SOLAR_ICONS = pkgs.fetchzip {
                url = "https://registry.npmjs.org/@iconify-json/solar/-/solar-1.2.17.tgz";
                hash = "sha256-gGhymGsI0P6QXeBfaQasVbLN4Y8DNTUsF4WIA/KAkc4=";
              };
              GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/${buildTools}/aapt2";
            };
        }
      );
    };
}
