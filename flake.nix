{
  description = "html-to-md — HTML-email-to-Markdown filter for aerc (Rust)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        html-to-md = pkgs.rustPlatform.buildRustPackage {
          pname = "html-to-md";
          version = "0.1.0";
          src = ./.;
          # `cargoLock.lockFile` reads Cargo.lock directly and derives the
          # per-crate hashes from it. Every dependency is a crates.io registry
          # package, so there is NO separate cargoHash/vendorHash to maintain —
          # it can never drift out of sync, and `just sync-flake` only has to
          # keep the version string aligned with Cargo.toml.
          cargoLock.lockFile = ./Cargo.lock;
          # `buildRustPackage` runs `cargo test` in the checkPhase by default,
          # exercising the decoder unit test in the sandbox.
          doCheck = true;

          meta = with pkgs.lib; {
            description = "Filter that converts vendor-noisy HTML email into clean Markdown for terminal viewing";
            homepage = "https://github.com/stubbedev/html-to-md";
            license = licenses.mit;
            mainProgram = "html-to-md";
            platforms = platforms.unix;
          };
        };
      in
      {
        packages = {
          default = html-to-md;
          html-to-md = html-to-md;
        };

        apps.default = {
          type = "app";
          program = "${html-to-md}/bin/html-to-md";
          meta = html-to-md.meta;
        };

        checks.build = html-to-md;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rust-analyzer
            rustfmt
            clippy
            just
            git
          ];
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}
