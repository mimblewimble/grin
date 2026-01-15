{
  description = "THE MIMBLEWIMBLE BLOCKCHAIN.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-22.11";
  };

  outputs = { self, nixpkgs, }:
    let
      forAllSystems = with nixpkgs;
        lib.genAttrs lib.systems.flakeExposed;

      nixpkgsFor = forAllSystems (system: import nixpkgs
        { inherit system; overlays = [ self.overlay ]; }
      );
    in
    {
      overlay = final: prev:
        with final;
        {
          grin = pkgs.rustPlatform.buildRustPackage {
            pname = "grin";
            version = "5.2.0-alpha.2";
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [ pkgs.llvmPackages_latest.clang ];
            buildInputs = [ pkgs.ncurses ];
            LIBCLANG_PATH =
              "${pkgs.llvmPackages_latest.libclang.lib}/lib";

            # do not let test results block the build process
            doCheck = false;
          };
        };

      packages = forAllSystems (
        system: {
          default = nixpkgsFor.${system}.grin;
        }
      );
    };
}
