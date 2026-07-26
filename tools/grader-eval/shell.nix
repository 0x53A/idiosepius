{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    cmake
    llvmPackages.clang
    llvmPackages.libclang
    pkg-config
    shaderc
    vulkan-headers
  ];

  buildInputs = with pkgs; [
    vulkan-loader
  ];

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.vulkan-loader ];
}
