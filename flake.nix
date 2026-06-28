{
  description = "A Vulkan based VIO";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    fenix.url = "github:nix-community/fenix";
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    flake-utils,
    fenix,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {inherit system;};
      craneLib = (crane.mkLib nixpkgs.legacyPackages.${system}).overrideToolchain fenix.packages.${system}.stable.toolchain;
      fenixPkgs = fenix.packages.${system};

      libs = with pkgs; [
        libxkbcommon
        wayland
        libGL
        vulkan-loader
      ];

      rustToolchain = fenixPkgs.stable.toolchain;

      # Include cargo files + shader files
      src = pkgs.lib.cleanSourceWith {
        filter = path: type: let
          baseName = baseNameOf path;
          cargoInclude =
            type
            == "directory"
            || baseName == "Cargo.toml"
            || baseName == "Cargo.lock"
            || baseName == "build.rs"
            || baseName == ".cargo-checksum.json"
            || pkgs.lib.hasSuffix ".rs" baseName
            || pkgs.lib.hasPrefix ".cargo" baseName;

          shaderInclude = pkgs.lib.hasSuffix ".slang" baseName;
        in
          cargoInclude || shaderInclude;

        src = ./.;
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        buildInputs = libs;
        nativeBuildInputs = [
          pkgs.shader-slang
          rustToolchain
        ];
      };

      photon-vio = craneLib.buildPackage (
        commonArgs
        // {
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        }
      );

      photon-vio-wrapped = pkgs.writeShellScriptBin "photon-vio" ''
        export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libs}:$LD_LIBRARY_PATH
        exec ${photon-vio}/bin/photon-vio "$@"
      '';
    in {
      checks = {
        inherit photon-vio;
      };

      packages.default = photon-vio;

      apps.default = flake-utils.lib.mkApp {
        drv = photon-vio-wrapped;
      };

      devShells.default = craneLib.devShell {
        checks = self.checks.${system};

        packages =
          libs
          ++ [
            rustToolchain
            pkgs.shader-slang
            pkgs.rust-analyzer
          ];

        shellHook = ''
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libs}:$LD_LIBRARY_PATH;
          export RUST_SRC_PATH=${rustToolchain}/lib/rustlib/src/rust/library;
        '';
      };
    });
}
