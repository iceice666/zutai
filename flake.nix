{
  description = "zutai Rust workspace";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, fenix, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # Stable toolchain with llvm-tools-preview for cargo-llvm-cov
          toolchain = fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "llvm-tools-preview"
            "rustc"
            "rustfmt"
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.cargo-llvm-cov
              pkgs.cargo-nextest
              pkgs.rust-analyzer
            ];

            RUST_BACKTRACE = "1";
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
