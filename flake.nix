{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };
  outputs = {
    nixpkgs,
    crane,
    ...
  }: let
    inherit (nixpkgs) lib;

    forEachSystem = cb:
      lib.genAttrs
      (lib.intersectLists lib.systems.flakeExposed lib.platforms.linux)
      (system: cb system nixpkgs.legacyPackages.${system});
  in {
    devShells = forEachSystem (_: pkgs: let
      craneLib = crane.mkLib pkgs;
    in {
      default = pkgs.callPackage ./shell.nix {inherit pkgs craneLib;};
    });
  };
}
