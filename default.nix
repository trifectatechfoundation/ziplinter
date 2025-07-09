{ pkgs ? import <nixpkgs> {} }:

pkgs.python3Packages.buildPythonPackage rec {
  pname = "ziplinter";
  version = "0.1.0";
  pyproject = true;

  # Currently, it uses the current directory as a source:
  src = ./.;

  # When the repository becomes public, we probably want something like this instead:

  # src = fetchFromGitHub {
  #   owner = "trifectatechfoundation";
  #   repo = "ziplinter";
  #   rev = "4cac21b3bbf83b71409c6747248b98ea6f8d5306";  # can be a commit hash or a release tag
  #   hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
  # };

  cargoDeps = pkgs.rust.packages.stable.rustPlatform.fetchCargoVendor {
    inherit pname version src;
    hash = "sha256-RjMp+9VfIalGcDGLdncYg/6KjIodR/9IMGQZw9/g2EM=";
  };

  buildAndTestSubdir = "ziplinter-python";

  nativeBuildInputs = with pkgs.rust.packages.stable.rustPlatform; [
    cargoSetupHook
    maturinBuildHook
  ];
}