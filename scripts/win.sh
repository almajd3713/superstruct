#!/usr/bin/env bash
# Build, test or run natively on Windows from WSL (CLAUDE.md §7).
#   scripts/win.sh build [cargo args]   e.g. build -p game --release
#   scripts/win.sh test  [cargo args]
#   scripts/win.sh run   [-p game --release] [-- game args]
# Uses the GNU toolchain (no MSVC linker needed) and a target dir on the Windows drive, because
# building into the \\wsl.localhost share is slow. Env vars reach the game via WSLENV.
set -euo pipefail
cd "$(dirname "$0")/.."

cmd=${1:?usage: scripts/win.sh build|test|run [args]}
shift
win_root=$(wslpath -w .)
profile=debug
for a in "$@"; do [[ $a == --release ]] && profile=release; done
win_home=$(cmd.exe /c "echo %USERPROFILE%" 2>/dev/null | tr -d '\r')
target_win="$win_home\\ss-target"
target_wsl=$(wslpath "$target_win")
toolchain=+stable-x86_64-pc-windows-gnu

case $cmd in
  build|test)
    cargo.exe $toolchain "$cmd" --manifest-path "$win_root\\Cargo.toml" --target-dir "$target_win" "$@"
    ;;
  run)
    cargo.exe $toolchain build -p game --manifest-path "$win_root\\Cargo.toml" --target-dir "$target_win" \
      $([[ $profile == release ]] && echo --release)
    export WSLENV="${WSLENV:+$WSLENV:}SUPERSTRUCT_LOG:SUPERSTRUCT_SETTINGS:SUPERSTRUCT_AUTOSHOT:SUPERSTRUCT_OVERLAY:SUPERSTRUCT_DEBUG_BOXES"
    "$target_wsl/$profile/game.exe"
    ;;
  *) echo "unknown command $cmd" >&2; exit 2 ;;
esac
