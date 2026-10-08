#!/usr/bin/env bash
# Look at Killing Floor's native code (the engine DLLs) with Ghidra, without
# its window. Read-only on the game install; everything Ghidra makes goes to
# work/re/ (gitignored). Rules: docs/reverse-engineering.md.
#
#   scripts/re.sh list DLL TEXT          exported names containing TEXT (fast, no analysis)
#   scripts/re.sh analyze DLL            import + analyse once (minutes; cached in work/re/projects)
#   scripts/re.sh decompile DLL TEXT [N] functions whose name contains TEXT -> work/re/out/
#   scripts/re.sh dump DLL               every function -> work/re/out/DLL-all.c (for grep)
#
# DLL is a file name in the install's System folder, e.g. Engine.dll.
# Enters the reverse-engineering dev shell (nix develop .#re) by itself.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z "${GHIDRA_HOME:-}" ]]; then
    exec nix develop "$repo#re" -c "$0" "$@"
fi

cmd="${1:-}"; dll="${2:-}"
[[ -n "$cmd" && -n "$dll" ]] || { sed -n '2,13p' "$0"; exit 1; }

root="${KF_ROOT:-$repo/references/killing_floor}"
bin="$root/System/$dll"
[[ -f "$bin" ]] || { echo "re: no such file: $bin" >&2; exit 1; }

work="$repo/work/re"
projects="$work/projects"
out="$work/out"
mkdir -p "$projects" "$out"
project="${dll%.*}"
headless="$GHIDRA_HOME/support/analyzeHeadless"
# Ghidra keeps settings and caches under the home folder; keep them in
# work/re too, so nothing is written outside the project.
mkdir -p "$work/home"
export HOME="$work/home" XDG_CONFIG_HOME="$work/home/.config" XDG_CACHE_HOME="$work/home/.cache"
export _JAVA_OPTIONS="-Duser.home=$work/home"

case "$cmd" in
    list)
        text="${3:?re: list needs TEXT}"
        # MSVC export names, demangled by Ghidra later; plain grep is enough here.
        objdump -p "$bin" | grep -E '^\s+\[ *[0-9]+\]' | grep -F -- "$text" || true
        ;;
    analyze)
        if [[ -d "$projects/$project.rep" ]]; then
            echo "re: $dll already analysed ($projects/$project.gpr)"
            exit 0
        fi
        # The project (Ghidra's database) lives in work/re; the DLL is only read.
        "$headless" "$projects" "$project" -import "$bin" -overwrite \
            -log "$work/$project-analyze.log" >/dev/null
        echo "re: analysed $dll -> $projects/$project.gpr (log: work/re/$project-analyze.log)"
        ;;
    decompile)
        text="${3:?re: decompile needs TEXT}"
        max="${4:-20}"
        [[ -d "$projects/$project.rep" ]] || "$0" analyze "$dll"
        safe="$(printf '%s' "$text" | tr -c 'A-Za-z0-9_.-' '_')"
        file="$out/$project-$safe.c"
        "$headless" "$projects" "$project" -process "$dll" -noanalysis -readOnly \
            -scriptPath "$repo/scripts/ghidra" -postScript DecompileFunctions.java "$text" "$file" "$max" \
            -log "$work/$project-decompile.log" 2>&1 | grep -E "DecompileFunctions:|ERROR" || true
        [[ -f "$file" ]] && echo "re: $file"
        ;;
    dump)
        "$0" decompile "$dll" "*" 1000000
        mv -f "$out/$project-_.c" "$out/$project-all.c" 2>/dev/null && echo "re: $out/$project-all.c"
        ;;
    *)
        sed -n '2,13p' "$0"; exit 1
        ;;
esac
