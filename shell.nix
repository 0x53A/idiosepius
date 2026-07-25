# Dev shell for idiosepius.
#
# eframe/winit dlopen the windowing and GL libraries at runtime, so they have
# to be on LD_LIBRARY_PATH — being in buildInputs is not enough.
#
#   nix-shell            # then: cargo run -p idiosepius-app
#   ./tools/shot.sh      # headless screenshots, needs the xvfb here

{ pkgs ? import <nixpkgs> { } }:

let
  # Loaded at runtime by winit (windowing) and glow (OpenGL).
  runtimeLibs = with pkgs; [
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    vulkan-loader
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    # Screenshotting the UI without a desktop session.
    xvfb-run
    mesa
  ];

  buildInputs = runtimeLibs ++ [ pkgs.sqlite ];

  # The database engine (turso) is pure Rust, so no system sqlite is required
  # to compile — this is here only for poking at the file by hand, which still
  # works because turso writes an ordinary SQLite database.
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
}
