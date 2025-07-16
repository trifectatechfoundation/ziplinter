{ pkgs ? import <nixpkgs> {} }:

pkgs.python3Packages.buildPythonPackage rec {
  pname = "ziplinter";
  version = "0.1.0";
  pyproject = true;

  src = pkgs.fetchFromGitHub {
    owner = "trifectatechfoundation";
    repo = "ziplinter";
    rev = "698922aff67194f511da0586433504cdf43fe965";
    hash = "sha256-YL41HUoQfc9StAAHBR0Gt7r5NFQsh6LjfdFfiYRNB4s=";
  };

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