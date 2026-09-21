{
  description = "mu — a minimal coding agent";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
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
              fileset = pkgs.lib.fileset.unions [ ./src ./Cargo.toml ./Cargo.lock ];
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
            packages = with pkgs; [ cargo rustc rustfmt clippy bash curl ripgrep python3 ];
          };
        });
    };
}
