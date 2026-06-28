{
  description = "A Vulkan based VIO";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
      };
      craneLib = crane.mkLib pkgs;
      libs = with pkgs; [
        libxkbcommon
        wayland
        libGL
        vulkan-loader
      ];

      # Include cargo files + shader files (crane's cleanCargoSource strips .wgsl/.slang)
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
        BuildInputs = libs;
        nativeBuildInputs = [
          pkgs.shader-slang
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
            pkgs.shader-slang
            pkgs.rust-analyzer
          ];
        shellHook = ''
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libs}:$LD_LIBRARY_PATH;
        '';
      };
    });
}
