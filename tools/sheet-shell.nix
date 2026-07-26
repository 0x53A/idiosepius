# Formula sheets only.
#
# `tools/build-sheet.sh` re-enters this shell when tectonic is not already on
# PATH. LaTeX is deliberately kept out of the main `shell.nix`: it is a large
# dependency that nothing in the build, the tests or the app needs, and direnv
# puts every `cd` into that shell.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  name = "idiosepius-sheet";
  packages = [
    pkgs.python3 # formula-sheet.py, and the rest of the pack tooling
    pkgs.tectonic # LaTeX, self-contained, no texlive install to maintain
  ];
}
