{
  description = "A Vulkan based SLAM";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
        config.android_sdk.accept_license = true;
      };
      arm64Pkgs = import nixpkgs {
        localSystem = {inherit system;};
        crossSystem = {
          config = "aarch64-unknown-linux-gnu";
        };
        config = {
          allowUnfree = true;
          android_sdk.accept_license = true;
        };
      };

      fenixPkgs = fenix.packages.${system};
      rustToolchain = fenixPkgs.stable.toolchain;
      arm64-rust = with fenixPkgs;
        combine [
          stable.toolchain
          targets.aarch64-unknown-linux-gnu.stable.rust-std
        ];
      android-rust = with fenixPkgs;
        combine [
          stable.toolchain
          targets.aarch64-linux-android.stable.rust-std
          targets.x86_64-linux-android.stable.rust-std
        ];

      craneLib = (crane.mkLib nixpkgs.legacyPackages.${system}).overrideToolchain rustToolchain;
      arm64-craneLib = (crane.mkLib arm64Pkgs).overrideToolchain arm64-rust;
      android-craneLib = (crane.mkLib nixpkgs.legacyPackages.${system}).overrideToolchain android-rust;

      androidEnv = pkgs.androidenv.override {licenseAccepted = true;};
      android = androidEnv.composeAndroidPackages {
        platformVersions = ["30"];
        buildToolsVersions = ["35.0.0"];
        includeNDK = true;
        ndkVersions = ["27.2.12479018"];
        platformToolsVersion = "latest";
      };

      android-sdk = [
        android.androidsdk
        pkgs.cargo-apk
        pkgs.pkg-config
        pkgs.cmake
        pkgs.ninja
        pkgs.jdk17
      ];

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
          imgInclude = pkgs.lib.hasSuffix ".jpg" baseName;
        in
          cargoInclude || shaderInclude || imgInclude;

        src = ./.;
      };

      libs = pkgs:
        with pkgs; [
          libxkbcommon
          wayland
          libGL
          vulkan-loader
          libX11
          libXcursor
          libXi
          libv4l
          linuxHeaders
          rustPlatform.bindgenHook
        ];
      libs_system = libs pkgs;

      commonArgs = pkgs: {
        inherit src;
        strictDeps = true;
        buildInputs = libs pkgs;
        nativeBuildInputs = [
          pkgs.shader-slang
        ];
      };
      commonArgsNative = commonArgs pkgs;
      commonArgsArm64 = commonArgs arm64Pkgs;

      photon-slam = craneLib.buildPackage (
        commonArgsNative
        // {
          cargoArtifacts = craneLib.buildDepsOnly commonArgsNative;
        }
      );
      arm64-photon-slam = arm64-craneLib.buildPackage (
        commonArgsArm64
        // {
          cargoArtifacts = craneLib.buildDepsOnly commonArgsArm64;
          CARGO_BUILD_TARGET = "aarch64-unknown-linux-gnu";
        }
      );

      photon-slam-wrapped = pkgs.writeShellScriptBin "photon-slam" ''
        export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libs_system}:$LD_LIBRARY_PATH
        exec ${photon-slam}/bin/photon-slam "$@"
      '';
    in {
      checks = {
        inherit photon-slam;
      };

      packages.default = photon-slam;
      packages.arm64-linux = arm64-photon-slam;

      apps.default = flake-utils.lib.mkApp {
        drv = photon-slam-wrapped;
      };

      devShells.default = craneLib.devShell {
        checks = self.checks.${system};

        packages =
          libs_system
          ++ commonArgsNative.nativeBuildInputs
          ++ [
            rustToolchain
            pkgs.bacon
            pkgs.rust-analyzer
            pkgs.clang-tools
          ];

        shellHook = ''
          export RUST_SRC_PATH=${rustToolchain}/lib/rustlib/src/rust/library;
        '';
      };

      devShells.android = android-craneLib.devShell {
        checks = self.checks.${system};

        packages =
          libs_system
          ++ commonArgsNative.nativeBuildInputs
          ++ android-sdk
          ++ [
            pkgs.bacon
            pkgs.rust-analyzer
            pkgs.clang-tools
          ];

        shellHook = ''
          export RUST_SRC_PATH=${android-rust}/lib/rustlib/src/rust/library;
          export ANDROID_HOME=${android.androidsdk}/libexec/android-sdk;
        '';
      };
    });
}
