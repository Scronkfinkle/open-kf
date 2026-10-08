# Looking at Killing Floor's native code

Most of KF's behaviour is in UnrealScript we can read. The rest (rendering,
lighting, sound mixing, physics) is compiled C++ in the engine DLLs. When a
mechanic is "in the native code", look at it with Ghidra (a tool that turns
machine code back into readable, C-like code) instead of guessing.

## Rules

1. **Nothing from the game goes into the repo.** Ghidra's projects, decompiled
   output and notes with pasted code stay in `work/re/` (gitignored). Never
   copy decompiled code into `src/`, docs, MODLOG or commit messages.
2. **Committed files get behaviour only; pointers go in `RE.md`.** Describe
   what KF does in plain words in `docs/DESIGN.md`, MODLOG and code comments
   ("volume is clamped to 0..1, multiplied by SoundVolume, clamped again"),
   with no DLL names, function names, addresses or "read with Ghidra"
   notes. Those details (where each rule is, so it can be checked again) go
   in `RE.md` at the repo root, which is gitignored (the whitelist does not
   list it) and stays on this machine. Committed text may say "details in
   the local RE.md". Write the Rust from the behaviour description.
3. **Only game mechanics.** Rendering, lighting, sound, physics, movement,
   game rules. Do not look into anti-cheat (VAC, PunkBuster), Steam/DRM,
   CD-key, login or master-server code. If a binary turns out to be packed or
   encrypted (e.g. a Steam DRM wrapper: a `.bind` section), stop and tell the
   user; do not unpack it.
4. **Read-only.** Read the DLLs where they are installed; never modify or
   patch them. `scripts/re.sh` keeps Ghidra's settings and caches in
   `work/re/home`, so nothing is written outside the project.
5. **Decompiled code is a strong hint, not proof.** Check the result against
   the game's behaviour (logged numbers, baked data, recordings) and say how
   it was checked. Label anything still uncertain as a guess.

## How

`scripts/re.sh` runs Ghidra without its window (it enters `nix develop .#re`,
which has Ghidra and objdump; the normal dev shell does not).

```
scripts/re.sh list Engine.dll Light          # exported names containing "Light" (instant)
scripts/re.sh analyze Engine.dll             # analyse once; cached in work/re/projects
scripts/re.sh decompile Engine.dll "Class::Method" 10
                                             # -> work/re/out/Engine-Class__Method.c
scripts/re.sh dump Engine.dll                # every function -> work/re/out/Engine-all.c
```

**Search the dumps first.** `work/re/out/<DLL>-all.c` holds every function
of the DLLs below, so `grep -n "LightRadius" work/re/out/*-all.c` finds
candidates in seconds; decompile a single function only when a dump is
missing or out of date.

Dumped (game and engine code): Core, Engine, ALAudio, ROEngine (Red
Orchestra's engine additions; KF is built on it), D3D9Drv, D3DDrv,
OpenGLDrv (renderers: lighting, projectors), Fire (procedural textures),
XGame, XInterface, Window, WinDrv (input), IpDrv (networking: read only the
game protocol, not master-server or CD-key code).

Not dumped, on purpose: other companies' libraries (d3dx9, binkw32,
ogg/vorbis, OpenAL, MSVCR71, dbghelp, pixomatic, IFC23), `steam_api.dll`
(Steam/DRM: off limits), the Massive ad client, and the editor/tools
(Editor.dll, KFEd.exe, UCC.exe; maybe later for map formats).

- `DLL` is a file in the install's `System` folder (`KF_ROOT`, default
  `references/killing_floor`).
- `decompile` matches the function name as Ghidra shows it, demangled
  (`Class::Method`), and writes up to N functions (default 20) to one file.
- The first `analyze` of a DLL takes a while (ALAudio.dll: about 40 s,
  Engine.dll about 2 minutes), then it is cached.
- The engine DLLs export their C++ names (Engine.dll has about 16,700), so
  `list` usually finds the right function by name. Unnamed helpers show up
  as `FUN_<address>`; decompile those by that name.

Tested 2026-10-08 on the sound engine's play function: the decompiled
output shows the volume rule found earlier by hand (details in RE.md).
