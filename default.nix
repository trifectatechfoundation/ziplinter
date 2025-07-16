{
  lib,
  fetchFromGitHub,
  python3,
  rustPlatform,
}:

python3.pkgs.buildPythonPackage rec {
  pname = "ziplinter";
  version = "0.1.0";
  pyproject = true;

  src = fetchFromGitHub {
    owner = "trifectatechfoundation";
    repo = "ziplinter";
    rev = "698922aff67194f511da0586433504cdf43fe965";
    hash = "sha256-YL41HUoQfc9StAAHBR0Gt7r5NFQsh6LjfdFfiYRNB4s=";
  };

  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit pname version src;
    hash = "sha256-RjMp+9VfIalGcDGLdncYg/6KjIodR/9IMGQZw9/g2EM=";
  };

  buildAndTestSubdir = "ziplinter-python";

  nativeBuildInputs = with rustPlatform; [
    cargoSetupHook
    maturinBuildHook
  ];

  meta = {
    description = "A zip file analyzer Python module";
    homepage = "https://github.com/trifectatechfoundation/ziplinter";
    license = with lib.licenses; [
      asl20
      mit
    ];
    maintainers = with lib.maintainers; [
      folkertdev
      michielp1807
      armijnhemel
    ];
    platforms = lib.platforms.all;
  };
}