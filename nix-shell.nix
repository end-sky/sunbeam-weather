{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [
    rustc cargo
    go
    pkg-config
    openssl
    libxkbcommon
    wayland
    libglvnd
    mesa
    vulkan-loader
    patchelf
    xorg.libX11
    xorg.libxcb
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
    openssl
    libxkbcommon
    wayland
    libglvnd
    mesa
    vulkan-loader
    xorg.libX11
    xorg.libxcb
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ]);

  shellHook = ''
    export PKG_CONFIG_PATH="${pkgs.lib.makeSearchPath "lib/pkgconfig" [ pkgs.openssl pkgs.libxkbcommon pkgs.wayland ] }:${pkgs.lib.makeSearchPath "lib/pkgconfig" [ pkgs.xorg.libX11 pkgs.xorg.libxcb pkgs.xorg.libXcursor pkgs.xorg.libXi pkgs.xorg.libXrandr ] }:$PKG_CONFIG_PATH"
    echo "Sunbeam Weather build shell: cargo + go ready"
    echo "Sunbeam Weather runtime libraries: configured"
  '';
}
