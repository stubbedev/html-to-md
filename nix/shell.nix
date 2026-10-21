{
  perSystem = { pkgs, self', ... }: {
    devShells.default = pkgs.mkShell {
      # Rolling Go channel: build, tests and lint all use the same newest
      # toolchain as the package.
      inputsFrom = [ self'.packages.default ];

      packages = with pkgs; [
        gotools
        gopls
        golangci-lint
        just
        git
      ];
    };

    formatter = pkgs.nixpkgs-fmt;
  };
}
