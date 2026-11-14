{ pkgs, ... }:
let
  toml = pkgs.formats.toml { };
  json = pkgs.formats.json { };
in
{
  projectRootFile = "flake.nix";

  programs = {
    rustfmt = {
      enable = true;
      edition = "2024";
    };
    nixfmt.enable = true;
    taplo = {
      enable = true;
      settings.formatting.column_width = 120;
    };
    ktfmt.enable = true;
    oxfmt.enable = true;
  };

  settings.formatter = {
    rustfmt.options = [
      "--config-path"
      (toString (
        toml.generate "rustfmt.toml" {
          max_width = 120;
          use_small_heuristics = "Max";
        }
      ))
    ];
    ktfmt.options = [ "--kotlinlang-style" ];
    oxfmt.options = [
      "--config"
      (toString (
        json.generate "oxfmtrc.json" {
          printWidth = 120;
          trailingComma = "none";
        }
      ))
    ];
  };

  settings.global.excludes = [
    "*.lock"
    "*.svg"
    "*.md"
    "*.properties"
    "*.xml"
    "LICENSE"
    ".envrc"
  ];
}
