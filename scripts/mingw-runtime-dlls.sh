#!/usr/bin/env bash
# Copy the mingw runtime DLLs a cross-built .exe needs into its run directory.
#
# Idea: the normal client is pure Rust and needs nothing, but the Tracy build
# links C++ (tracy-client-sys compiles TracyClient.cpp), which pulls in
# libstdc++ -- which in turn pulls in libgcc. Windows refuses to start the exe
# if any of them is missing, one error box at a time, so this walks the whole
# import closure instead of naming a fixed list of DLLs.
#
# Only non-system DLLs are followed: anything resolvable inside the toolchain's
# own lib directory is ours to ship, everything else (KERNEL32, msvcrt, the
# api-ms-win-* stubs) belongs to Windows.
#
# Usage: scripts/mingw-runtime-dlls.sh <exe> <dest-dir>
set -euo pipefail

exe=${1:?usage: mingw-runtime-dlls.sh <exe> <dest-dir>}
dest=${2:?usage: mingw-runtime-dlls.sh <exe> <dest-dir>}

# Overridable so the test can substitute recording stand-ins for the toolchain
# (scripts/test_mingw_runtime_dlls.sh) — the logic worth testing is the walk,
# not whether mingw is installed on the machine running the tests.
OBJDUMP=${MINGW_OBJDUMP:-x86_64-w64-mingw32-objdump}
CXX=${MINGW_CXX:-x86_64-w64-mingw32-g++}

command -v "$OBJDUMP" >/dev/null 2>&1 || {
    echo "mingw-runtime-dlls: need x86_64-w64-mingw32-objdump (binutils-mingw-w64-x86-64)" >&2
    exit 1
}

# Ask the compiler that built the exe where its runtime lives, rather than
# hardcoding a version-stamped path: the win32 and posix g++ variants ship
# different DLLs, and picking the wrong one is a subtle runtime mismatch.
libdir=$(dirname "$("$CXX" -print-file-name=libstdc++-6.dll 2>/dev/null)")
[ -d "$libdir" ] || { echo "mingw-runtime-dlls: cannot locate the mingw runtime directory" >&2; exit 1; }

imports() { "$OBJDUMP" -p "$1" 2>/dev/null | awk '/DLL Name:/ {print $3}'; }

# No associative array and no shrinking array on purpose: this script has to
# run on the machine that does the cross-build, and macOS still ships bash 3.2,
# where `declare -A` is a syntax error and `${#queue[@]}` on an emptied array
# trips `set -u`. So "already seen" is a delimited string we grep with `case`,
# and the queue is walked by a moving index instead of being consumed.
seen="|"
queue=("$exe")
head=0
copied=0

while [ "$head" -lt "${#queue[@]}" ]; do
    current=${queue[$head]}
    head=$((head + 1))
    while read -r dll; do
        [ -n "$dll" ] || continue
        case "$seen" in *"|$dll|"*) continue ;; esac
        seen="$seen$dll|"
        src="$libdir/$dll"
        # Not in the toolchain's lib dir => a Windows system DLL, not ours.
        [ -f "$src" ] || continue
        cp -f "$src" "$dest/"
        echo "  $dll"
        copied=$((copied + 1))
        queue=("${queue[@]}" "$src")
    done < <(imports "$current")
done

if [ "$copied" -eq 0 ]; then
    echo "  (none needed -- the exe links no mingw runtime)"
fi
