{
  description = "html-to-md — aerc email→Markdown filters (html/plain/calendar) in one Go binary";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        goRolling = pkgs.go_latest;

        goBuilder = pkgs.buildGoModule.override { go = goRolling; };

        html-to-md = goBuilder {
          pname = "html-to-md";
          version = "0.1.1";
          src = ./go;
          # Build with the same rolling toolchain as the dev shell so the
          # package and local development never disagree on language version.
          # Dependencies (x/net/html + go-runewidth) are vendored into
          # `go/vendor`, so there is no vendorHash to maintain and the sandbox
          # never needs a module proxy.
          vendorHash = null;
          # The check phase runs `go test ./...`.
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
            # Rolling Go channel: build, tests and lint all use the same
            # newest toolchain as the package above.
            goRolling
            gotools
            gopls
            golangci-lint
            just
            git
          ];
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}
