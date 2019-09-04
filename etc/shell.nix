# Run `nix-shell` to be able
# to build Grin on NixOS.
{ pkgs ? import <nixpkgs> {} }:

pkgs.stdenv.mkDerivation {
  name = "grin";

  buildInputs = with pkgs; [
    ncurses cmake clang
  ];

  shellHook = ''
      LD_LIBRARY_PATH=${pkgs.ncurses}/lib/:$LD_LIBRARY_PATH
      LD_LIBRARY_PATH=${pkgs.zlib}/lib/:$LD_LIBRARY_PATH
      LIBRARY_PATH=${pkgs.zlib}/lib/:$LIBRARY_PATH
      LD_LIBRARY_PATH=${pkgs.llvmPackages.libclang}/lib/:$LD_LIBRARY_PATH
      CXX=${pkgs.clang}/bin/clang++
  '';
}
