{
  description = "Open KF Flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = { self, nixpkgs }:
  let
    pkgs = import nixpkgs { system = "x86_64-linux";};

    stdInputs = [
        # Rust
        pkgs.cargo
        pkgs.rustc
        pkgs.pkg-config
        pkgs.glib

        # Python
        pkgs.python314

        # Extra python packages
        pkgs.python313Packages.tqdm

        # version control
        pkgs.git
        pkgs.git-lfs

    ];
    devInputs = [
        pkgs.rustfmt
        pkgs.clippy
        pkgs.rust-analyzer

        # Video recording (F9) pipes frames to ffmpeg
        pkgs.ffmpeg
    ];

    # System libraries Bevy needs on Linux: audio, input devices, GPU, windowing
    bevyInputs = [
        pkgs.alsa-lib
        pkgs.udev
        pkgs.vulkan-loader
        pkgs.libxkbcommon
        pkgs.wayland
        pkgs.libx11
        pkgs.libxcursor
        pkgs.libxi
        pkgs.libxrandr
    ];
  in
  {
    devShells."x86_64-linux".default = pkgs.mkShell {
       buildInputs = stdInputs ++ devInputs ++ bevyInputs;

      # Vulkan and the window system are loaded at runtime (dlopen), so the
      # game must find them. They are written into the game binary's own
      # library path (rpath) instead of exporting LD_LIBRARY_PATH: an
      # exported path leaks into every program started from this shell
      # (e.g. VS Code built against the system's older glibc failed to start
      # with this shell's alsa-lib).
      RUSTFLAGS = "-C link-arg=-Wl,-rpath,${pkgs.lib.makeLibraryPath bevyInputs}";

      # Rust stdlib for language servers
      RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

    };

  };
}
