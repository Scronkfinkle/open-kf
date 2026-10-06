#!/usr/bin/env bash
# Run open-kf on a virtual X display (Xvfb), so no window opens on your screen.
# All arguments go to the game. Use --screenshot or --frames so it quits.
#
#   scripts/headless.sh --map KF-WestLondon --camera -3110,1313,-3768,-3.72,0 --screenshot 60
#
# Settings (environment variables):
#   HEADLESS_SCREEN=1920x1080x24  size of the virtual display
#   HEADLESS_TIMEOUT=300          kill the game after this many seconds
#   HEADLESS_SOFTWARE=1           draw on the CPU (Mesa lavapipe) instead of the GPU
#   HEADLESS_ICD=path.json        Vulkan driver file used by HEADLESS_SOFTWARE
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

# Needs the dev shell for Xvfb and the library path; enter it if we are outside.
if [[ -z "${OPEN_KF_LIBS:-}" ]] || ! command -v Xvfb >/dev/null; then
    exec nix develop "$repo" -c "$0" "$@"
fi

screen="${HEADLESS_SCREEN:-1920x1080x24}"
timeout_s="${HEADLESS_TIMEOUT:-300}"

cargo build --release

# Xvfb picks a free display number and writes it to fd 3.
display_file="$(mktemp)"
Xvfb -displayfd 3 -screen 0 "$screen" -nolisten tcp 3>"$display_file" 2>/dev/null &
xvfb_pid=$!
trap 'kill "$xvfb_pid" 2>/dev/null; wait "$xvfb_pid" 2>/dev/null; rm -f "$display_file"' EXIT

for _ in $(seq 50); do
    [[ -s "$display_file" ]] && break
    kill -0 "$xvfb_pid" 2>/dev/null || { echo "headless: Xvfb failed to start" >&2; exit 1; }
    sleep 0.1
done
display_num="$(tr -d '[:space:]' <"$display_file")"
[[ -n "$display_num" ]] || { echo "headless: Xvfb gave no display number" >&2; exit 1; }
echo "headless: Xvfb on :$display_num ($screen)"

game_env=(DISPLAY=":$display_num" LD_LIBRARY_PATH="$OPEN_KF_LIBS")
if [[ -n "${HEADLESS_SOFTWARE:-}" ]]; then
    game_env+=(VK_ICD_FILENAMES="${HEADLESS_ICD:-/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json}")
    echo "headless: software rendering (lavapipe)"
fi

# Unset WAYLAND_DISPLAY so the game uses the virtual X display, not your desktop.
status=0
env -u WAYLAND_DISPLAY "${game_env[@]}" timeout "$timeout_s" target/release/open-kf "$@" || status=$?
[[ $status -eq 124 ]] && echo "headless: game killed after ${timeout_s}s timeout" >&2
exit "$status"
