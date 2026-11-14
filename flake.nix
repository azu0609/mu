{
  description = "mu — a minimal coding agent";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  # A Rust toolchain with Android's standard libraries, for the app.
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      each = nixpkgs.lib.genAttrs systems;
    in {
      packages = each (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "mu";
            version = "0.1.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [ ./crates ./Cargo.toml ./Cargo.lock ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            stripAllList = [ "bin" ];
            postInstall = ''
              wrapProgram $out/bin/mu --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.bash pkgs.curl pkgs.ripgrep ]}
            '';
          };
        });
      devShells = each (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [ cargo rustc rust-analyzer rustfmt clippy bash curl ripgrep ];
          };
        } // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          # The Android app (app/): Gradle builds the Rust core with cargo-ndk.
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
                extensions = [ "rust-src" "rust-analyzer" ];
                targets = [ "aarch64-linux-android" "x86_64-linux-android" ];
              };
              jdk = pkgs.jdk21;
            in
            pkgs.mkShell {
              packages = [ rust pkgs.cargo-ndk android.androidsdk jdk pkgs.gradle_9 ];
              ANDROID_HOME = sdk;
              ANDROID_SDK_ROOT = sdk;
              ANDROID_NDK_HOME = "${sdk}/ndk/${ndk}";
              JAVA_HOME = jdk.home;
              # The aapt2 AGP fetches from Maven is a generic Linux binary that does not run on NixOS.
              GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/${buildTools}/aapt2";
            };
        });
    };
}
