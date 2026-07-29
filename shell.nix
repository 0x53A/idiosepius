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
    # cpal, under the apteronotus soundscape player, opens ALSA at runtime.
    alsa-lib
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    vulkan-loader
    # rfd's xdg-portal backend dlopens libdbus-1.so.3 to talk to the file
    # portal. Without it the picker cannot open, silently falls through to a
    # zenity that is not installed either, and returns "nothing was picked" —
    # which is indistinguishable from the user pressing Cancel. Import and
    # export then do nothing at all, with no error. Keep dbus on this list.
    dbus.lib
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    # Screenshotting the UI without a desktop session.
    xvfb-run
    mesa
  ];

  # alsa-lib is in runtimeLibs and therefore also a buildInput: cpal's
  # alsa-sys asks pkg-config for it at compile time, not only at load time.
  buildInputs = runtimeLibs ++ [ pkgs.sqlite ];

  # The database engine (turso) is pure Rust, so no system sqlite is required
  # to compile — this is here only for poking at the file by hand, which still
  # works because turso writes an ordinary SQLite database.
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
}
