{
  description = "Bare metal ESP32-C3 dev environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nixpkgs-esp-dev.url = "github:mirrexagon/nixpkgs-esp-dev";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    nixpkgs-esp-dev,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (import rust-overlay)
            (import "${nixpkgs-esp-dev}/overlay.nix")
          ];
          config = {
            permittedInsecurePackages = [
              "python3.13-ecdsa-0.19.1"
            ];
          };
        };
        rust-esp32c3 = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            esptool
            rust-esp32c3
            nodejs
            esp-idf-riscv
            gdb
            espup
            espflash
            ldproxy
            cargo-generate
            libclang
            esp-generate
            probe-rs-tools
          ];
          shellHook = ''
            export NPM_CONFIG_PREFIX=$(pwd)/.npm-packages
            export PATH=$NPM_CONFIG_PREFIX/bin:$PATH
            export LIBCLANG_PATH=${pkgs.libclang.lib}/lib/
          '';
        };
      }
    );
}
