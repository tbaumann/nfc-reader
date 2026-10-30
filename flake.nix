{
  description = "ESP32 (xtensa) + Rust/embassy dev shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        # rustup is only the shim: the actual `esp` toolchain (xtensa fork) is
        # installed into ~/.rustup by espup, and rust-toolchain.toml selects it.
        # gcc is needed as the *host* `cc` for build scripts.
        packages = with pkgs; [
          rustup
          espup
          espflash
          gcc
          httpie
        ];

        shellHook = ''
          if [ -f "$HOME/export-esp.sh" ]; then
            . "$HOME/export-esp.sh"   # xtensa-esp-elf-gcc + LIBCLANG_PATH
          else
            echo "!! ~/export-esp.sh missing - run: espup install -t esp32" >&2
          fi
        '';
      };
    };
}
