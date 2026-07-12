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
          fenixPkgs = fenix.packages.${system};
          # Stable host tools plus the WebAssembly standard library used by the
          # browser kernel and `zutai web build`.
          toolchain = fenixPkgs.combine [
            (fenixPkgs.stable.withComponents [
              "cargo"
              "clippy"
              "llvm-tools-preview"
              "rustc"
              "rustfmt"
            ])
            fenixPkgs.targets.wasm32-unknown-unknown.stable.rust-std
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.binaryen
              pkgs.cargo-llvm-cov
              pkgs.cargo-nextest
              pkgs.just
              pkgs.rust-analyzer
              pkgs.llvmPackages.clang
              pkgs.llvmPackages.llvm
              pkgs.wasm-bindgen-cli
              pkgs.wrangler
            ];

            RUST_BACKTRACE = "1";
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
