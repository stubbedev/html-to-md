{
  perSystem = { lib, pkgs, self', ... }:
    let
      goBuilder = pkgs.buildGoModule.override { go = pkgs.go_latest; };
    in
    {
      packages = {
        default = self'.packages.html-to-md;
        html-to-md = goBuilder {
          pname = "html-to-md";
          version = "0.1.1";
          src = ../go;
          # Build with the same rolling toolchain as the dev shell so the
          # package and local development never disagree on language version.
          # Dependencies (x/net/html + go-runewidth) are vendored into
          # `go/vendor`, so there is no vendorHash to maintain and the sandbox
          # never needs a module proxy.
          vendorHash = null;
          # The check phase runs `go test ./...`.
          doCheck = true;

          meta = {
            description = "Filter that converts vendor-noisy HTML email into clean Markdown for terminal viewing";
            homepage = "https://github.com/stubbedev/html-to-md";
            license = lib.licenses.mit;
            mainProgram = "html-to-md";
            platforms = lib.platforms.unix;
          };
        };
      };

      apps.default = {
        type = "app";
        program = lib.getExe self'.packages.html-to-md;
        inherit (self'.packages.html-to-md) meta;
      };

      checks.build = self'.packages.html-to-md;
    };
}
