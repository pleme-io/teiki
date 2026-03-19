{
  description = "teiki (定期) — cross-platform scheduled task management";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, substrate, ... }: let
    supportedSystems = ["aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux"];
    forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems f;
  in {
    packages = forAllSystems (system: let
      pkgs = import nixpkgs { inherit system; };
    in {
      default = pkgs.rustPlatform.buildRustPackage {
        pname = "teiki";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoLock.outputHashes = {
          "shikumi-0.1.0" = "sha256-i5jXXRJmAtmwQ9XXjT2rP59uu3rDg+nEl9PobCAbo60=";
        };
        meta = {
          description = "Cross-platform scheduled task management";
          license = pkgs.lib.licenses.mit;
          mainProgram = "teiki";
        };
      };
    });

    homeManagerModules.default = import ./module;

    overlays.default = final: prev: {
      teiki = self.packages.${final.system}.default;
    };

    devShells = forAllSystems (system: let
      pkgs = import nixpkgs { inherit system; };
    in {
      default = pkgs.mkShellNoCC {
        packages = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer
        ];
      };
    });
  };
}
