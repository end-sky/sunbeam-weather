{
  description = "Sunbeam Weather — Linux weather metasearch desktop app";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustc cargo go pkg-config openssl
            libxkbcommon wayland
            libglvnd mesa vulkan-loader
            xorg.libX11 xorg.libxcb xorg.libXcursor xorg.libXi xorg.libXrandr
            patchelf
          ];

          # Cargo links the Rust program against Nix-store libraries, but the
          # dynamic loader does not search those directories automatically.
          # Put the native Linux GUI dependencies on the runtime path as well.
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
        };
      });
    };
}
