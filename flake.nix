{
  description = "Open KF Flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    # Builds Rust projects in Nix, caching dependencies separately (release builds)
    crane.url = "github:ipetkov/crane";
    # Prebuilt Rust toolchains with extra targets (the Windows build)
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, rust-overlay }:
  let
    pkgs = import nixpkgs {
      system = "x86_64-linux";
      overlays = [ (import rust-overlay) ];
    };

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

        # Virtual X display for scripts/headless.sh (runs the game off-screen)
        pkgs.xvfb
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

    # Release builds (docs/DESIGN.md, "Release builds"). Only the Rust
    # sources and Cargo files go into the build.
    craneLib = crane.mkLib pkgs;
    src = craneLib.cleanCargoSource ./.;

    # R1: the game for Linux/Nix. The runtime libraries go into the
    # binary's rpath, as in the dev shell.
    linuxArgs = {
      inherit src;
      pname = "open-kf";
      strictDeps = true;
      doCheck = false;
      nativeBuildInputs = [ pkgs.pkg-config pkgs.patchelf ];
      buildInputs = bevyInputs;
    };
    open-kf = craneLib.buildPackage (linuxArgs // {
      cargoArtifacts = craneLib.buildDepsOnly linuxArgs;
      # Remove debug symbols (about 40% of the file).
      stripAllList = [ "bin" ];
      postFixup = ''
        patchelf --add-rpath ${pkgs.lib.makeLibraryPath bevyInputs} $out/bin/open-kf
      '';
    });

    # R2: the game for Windows, cross-compiled with MinGW. Rust comes
    # prebuilt with the Windows standard library (rust-overlay).
    mingw = pkgs.pkgsCross.mingwW64;
    windowsTarget = "x86_64-pc-windows-gnu";
    windowsCraneLib = (crane.mkLib pkgs).overrideToolchain (p:
      p.rust-bin.stable.latest.minimal.override { targets = [ windowsTarget ]; });
    windowsArgs = {
      inherit src;
      pname = "open-kf";
      strictDeps = true;
      doCheck = false;
      CARGO_BUILD_TARGET = windowsTarget;
      CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${mingw.stdenv.cc}/bin/${mingw.stdenv.cc.targetPrefix}cc";
      TARGET_CC = "${mingw.stdenv.cc}/bin/${mingw.stdenv.cc.targetPrefix}cc";
      TARGET_AR = "${mingw.stdenv.cc.bintools.bintools}/bin/${mingw.stdenv.cc.targetPrefix}ar";
      # The Windows threads library is linked from the MinGW package.
      CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${mingw.windows.pthreads}/lib";
      depsBuildBuild = [ mingw.stdenv.cc ];
    };
    windows = windowsCraneLib.buildPackage (windowsArgs // {
      cargoArtifacts = windowsCraneLib.buildDepsOnly windowsArgs;
      nativeBuildInputs = [ pkgs.zip ];
      # Remove debug symbols, then zip the exe with the licences for a
      # release. The exe needs only DLLs that come with Windows.
      postInstall = ''
        ${mingw.stdenv.cc.targetPrefix}strip $out/bin/open-kf.exe
        mkdir -p zip/open-kf
        cp $out/bin/open-kf.exe zip/open-kf/
        cp ${./LICENSE-MIT} zip/open-kf/LICENSE-MIT
        cp ${./LICENSE-APACHE} zip/open-kf/LICENSE-APACHE
        (cd zip && zip -r $out/open-kf-windows-x86_64.zip open-kf)
      '';
    });

    # R3: `nix run .#flatpak` builds work/flatpak/open-kf.flatpak with
    # flatpak-builder (Flatpak apps use Flatpak's runtime, not Nix's
    # libraries). Downloads and build state stay in work/flatpak/.
    flatpakAppId = "io.github.scronkfinkle.OpenKF";
    build-flatpak = pkgs.writeShellApplication {
      name = "build-flatpak";
      runtimeInputs = [ pkgs.flatpak pkgs.flatpak-builder pkgs.coreutils ];
      text = ''
        [ -f flake.nix ] || { echo "run this from the project folder" >&2; exit 1; }
        out="$PWD/work/flatpak"
        mkdir -p "$out"
        # Keep runtimes and the SDK out of your own flatpak folder.
        export FLATPAK_USER_DIR="$out/user"
        flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
        # Only git-tracked files (no target/, no game files).
        rm -rf "$out/src"
        cp -r ${self} "$out/src"
        chmod -R u+w "$out/src"
        flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse \
          --state-dir="$out/state" --repo="$out/repo" --force-clean \
          "$out/build" "$out/src/packaging/flatpak/${flatpakAppId}.yml"
        flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
          "$out/repo" "$out/open-kf.flatpak" ${flatpakAppId}
        echo "built $out/open-kf.flatpak"
      '';
    };
  in
  {
    apps."x86_64-linux".flatpak = {
      type = "app";
      program = "${build-flatpak}/bin/build-flatpak";
    };

    packages."x86_64-linux" = {
      inherit open-kf windows;
      default = open-kf;
    };

    devShells."x86_64-linux".default = pkgs.mkShell {
       buildInputs = stdInputs ++ devInputs ++ bevyInputs;

      # Vulkan and the window system are loaded at runtime (dlopen), so the
      # game must find them. They are written into the game binary's own
      # library path (rpath) instead of exporting LD_LIBRARY_PATH: an
      # exported path leaks into every program started from this shell
      # (e.g. VS Code built against the system's older glibc failed to start
      # with this shell's alsa-lib).
      RUSTFLAGS = "-C link-arg=-Wl,-rpath,${pkgs.lib.makeLibraryPath bevyInputs}";

      # The same library path as a plain variable, for scripts/headless.sh.
      # It sets LD_LIBRARY_PATH from this for the game process only.
      OPEN_KF_LIBS = pkgs.lib.makeLibraryPath bevyInputs;

      # Rust stdlib for language servers
      RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

    };

    # Reverse-engineering shell for scripts/re.sh (docs/reverse-engineering.md):
    # Ghidra (headless) and binutils' objdump, kept out of the default shell
    # because Ghidra and its Java runtime are large.
    devShells."x86_64-linux".re = pkgs.mkShell {
      buildInputs = [ pkgs.ghidra pkgs.binutils ];
      GHIDRA_HOME = "${pkgs.ghidra}/lib/ghidra";
    };

  };
}
