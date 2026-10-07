
## Project
A complete rewrite of the killing floor games engine to play with its assets

## Hard rules, never break these

1. **Never write game assets, decompiled code, or extracted game files into this
   repository.** They stay on this machine, untracked. If you need to read game
   data, read it from the install path at runtime or extract it to a gitignored
   folder.
2. **`.gitignore` is a whitelist.** It ignores everything and includes only
   source files. Do not switch it to a normal ignore list.
3. **Do not run `git commit` unless I asked.** Stage nothing beyond what the
   current task requires.
4. **Do not touch anything outside this project folder** unless I explicitly name
   the path. This includes my game installs: read them, never write to them.
5. **No anti-cheat or DRM bypassing.** If something touches anti-cheat or DRM,
   stop and tell me. Switching anti-cheat off through the game's own option is
   fine. Bypassing anti-cheat is never fine.
6. **Never put credentials in the repo or in any file you can read:** no API keys,
   no tokens, no passwords.

## How to work

- **Plan before code.** For anything more than a small fix, write the plan to
  `docs/DESIGN.md` first, then implement one step at a time.
- **One thing at a time.** Do not bundle unrelated changes. I want to be able to
  revert a single step.
- **Log, don't look.** You cannot see the game. Instrument instead: write
  positions, counts, timings, and state transitions to a log file so you can
  verify from the numbers. Do not try to visually inspect the game.
- **Tell me how to test it.** After every change, say the exact command to run and
  what I should see. "Done" without a test procedure is not done.
- **Ask before large refactors.** If you think the architecture is wrong, say so
  and explain, then wait for me.
- **Explain in plain language.** I am not the programmer here. If you use a term,
  explain it the first time.

## Running the game for tests

- **Use `scripts/headless.sh` for your own test runs and screenshots**, not
  `cargo run`. It takes the same options as the game, runs it on a virtual
  display (Xvfb) so no window opens on my screen, and enters `nix develop`
  by itself. Example:
  `scripts/headless.sh --map KF-WestLondon --fly --camera X,Y,Z,YAW,PITCH --mute --screenshot 60`
  (PNG goes to `work/screenshots/`).
- **Always pass `--mute`.** The virtual display has no screen but the sound
  still plays on my speakers. Sound is still mixed and logged when muted
  (`sound_play` lines), so sound tests can use `--mute` too. Leave it off
  only if I ask to hear something.
- Always pass `--screenshot` or `--frames` so the run ends. `HEADLESS_TIMEOUT`
  (default 300 s) kills a stuck run; `HEADLESS_SOFTWARE=1` draws without the GPU.
- The mouse does nothing on the virtual display: drive tests with `--camera`,
  `--input` and `--autowalk`.
- Logs and numbers stay the main check. You may look at screenshots to
  confirm visual fixes. Saved views are in `docs/test-views.md`.

## Honesty

- If something is not tested, write **"not tested"**. Never imply you verified
  something you didn't.
- If you are not sure, say you are not sure. A confident wrong answer costs me
  hours.
- If you hit something you cannot solve after two real attempts, stop and write
  up `STATUS.md` (see `STATUS-handoff.md`) instead of trying variations at random.
- Record failures, not just successes. A dead end I can see is worth more than a
  dead end I have to watch you repeat.

## Keep these files updated

- `MODLOG.md`: add an entry after every change. Template in
  `MODLOG-template.md`.
- `docs/DESIGN.md`: how the project works, in plain language. Update when the
  architecture changes, not on every commit.
- `WORK_LOG.md`: the "what works / what doesn't work" list. Test before you claim
  something works.

## Environment

- OS: [Linux, with windows support eventually]
- Game A: Killing Floor, installed at references/killing_floor
- Language and version: [e.g. Rust stable ]
- Agent: [Claude Code]
