#!/usr/bin/env bash
# Exercises scripts/mingw-runtime-dlls.sh WITHOUT mingw and WITHOUT a Windows exe.
#
# IDEA. What the script does is walk an import graph, so the graph is the fixture:
# a recording stand-in for `objdump -p` (MINGW_OBJDUMP) answers from a table of
# text files, and a stand-in for `g++ -print-file-name` (MINGW_CXX) points at a
# fake toolchain lib dir holding fake DLLs. That makes every interesting property
# testable on a machine with no cross-toolchain at all.
#
#   T1 the script parses under bash 3.2 — the shell macOS ships, i.e. the shell of
#      the machine that cross-builds the exe
#   T2 no bash-4-only construct is left in the source
#   T3 the transitive closure is copied and Windows system DLLs are not
#   T4 an import cycle terminates instead of looping forever
#   T5 an exe with no mingw imports reports "(none needed"
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
S="${HERE}/mingw-runtime-dlls.sh"
TMP="$(mktemp -d /tmp/mingw-dlls-test.XXXXXX)"
FAIL=0
ok()  { echo "$1 ok - $2"; }
bad() { echo "$1 FAIL - $2"; FAIL=1; }

# --- fake toolchain -----------------------------------------------------------
# imports table: ${IMPORTS}/<basename> holds the DLL names that file imports.
BIN="${TMP}/bin"; LIB="${TMP}/lib"; IMPORTS="${TMP}/imports"; DEST="${TMP}/dest"
mkdir -p "$BIN" "$LIB" "$IMPORTS" "$DEST"
cat > "${BIN}/objdump" <<'EOF'
#!/bin/sh
# usage: objdump -p <file>; prints one "DLL Name:" line per recorded import
f="${IMPORTS}/$(basename "$2")"
[ -f "$f" ] || exit 0
while read -r dll; do [ -n "$dll" ] && echo "	DLL Name: $dll"; done < "$f"
EOF
cat > "${BIN}/gxx" <<'EOF'
#!/bin/sh
echo "${LIB}/libstdc++-6.dll"
EOF
chmod +x "${BIN}/objdump" "${BIN}/gxx"
export IMPORTS LIB
export MINGW_OBJDUMP="${BIN}/objdump" MINGW_CXX="${BIN}/gxx"

# the toolchain's own DLLs (ours to ship); KERNEL32 deliberately absent here
: > "${LIB}/libstdc++-6.dll"
: > "${LIB}/libgcc_s_seh-1.dll"
: > "${LIB}/libwinpthread-1.dll"

EXE="${TMP}/client.exe"; : > "$EXE"
printf 'libstdc++-6.dll\nKERNEL32.dll\napi-ms-win-crt-runtime-l1-1-0.dll\n' > "${IMPORTS}/client.exe"
printf 'libgcc_s_seh-1.dll\nmsvcrt.dll\n' > "${IMPORTS}/libstdc++-6.dll"
printf 'libwinpthread-1.dll\n' > "${IMPORTS}/libgcc_s_seh-1.dll"
printf 'libgcc_s_seh-1.dll\n' > "${IMPORTS}/libwinpthread-1.dll"   # cycle, on purpose

echo "== T1 parses under bash 3.2 (the shell macOS ships) =="
SH32=""
for cand in /bin/bash /usr/bin/bash; do
    [ -x "$cand" ] || continue
    case "$("$cand" --version 2>/dev/null | head -1)" in *"version 3."*) SH32="$cand"; break ;; esac
done
if [ -n "$SH32" ]; then
    if "$SH32" -n "$S" 2>"${TMP}/parse.err"; then
        ok T1 "$SH32 parses the script"
    else
        bad T1 "$SH32 cannot parse the script: $(cat "${TMP}/parse.err")"
    fi
else
    ok T1 "skipped - no bash 3.x on this machine (T2 still covers the constructs)"
fi

echo "== T2 no bash-4-only construct in the source =="
BAD4=""
# Comment lines are stripped first: this file's own prose names the constructs.
CODE="$(grep -v '^[[:space:]]*#' "$S")"
printf '%s' "$CODE" | grep -q 'declare -A' && BAD4="${BAD4} declare-A"
printf '%s' "$CODE" | grep -q 'mapfile\|readarray' && BAD4="${BAD4} mapfile/readarray"
printf '%s' "$CODE" | grep -q '\${[A-Za-z_][A-Za-z_0-9]*,,' && BAD4="${BAD4} case-conversion"
printf '%s' "$CODE" | grep -q '&>>' && BAD4="${BAD4} appending-&>>"
if [ -z "$BAD4" ]; then ok T2 "bash 3.2 compatible constructs only"
else bad T2 "bash-4-only constructs present:${BAD4}"; fi

echo "== T3 transitive closure copied, system DLLs skipped =="
T3FAIL=0
OUT="$(${SH32:-bash} "$S" "$EXE" "$DEST" 2>&1)"; RC=$?
if [ "$RC" -ne 0 ]; then bad T3 "exit $RC: $OUT"; T3FAIL=1; fi
for want in libstdc++-6.dll libgcc_s_seh-1.dll libwinpthread-1.dll; do
    [ -f "${DEST}/${want}" ] || { bad T3 "missing $want in the run dir"; T3FAIL=1; }
done
for unwanted in KERNEL32.dll msvcrt.dll api-ms-win-crt-runtime-l1-1-0.dll; do
    [ -e "${DEST}/${unwanted}" ] && { bad T3 "copied the Windows system DLL $unwanted"; T3FAIL=1; }
done
[ "$T3FAIL" -eq 0 ] && ok T3 "3 runtime DLLs copied, 3 system DLLs left alone"

echo "== T4 an import cycle terminates =="
# libwinpthread -> libgcc -> libwinpthread: without "seen" this never returns.
if [ "$(printf '%s\n' "$OUT" | grep -c 'libgcc_s_seh-1.dll')" -eq 1 ]; then
    ok T4 "each DLL is visited once"
else
    bad T4 "libgcc_s_seh-1.dll reported more than once: $OUT"
fi

echo "== T5 an exe with no mingw imports says so =="
PURE="${TMP}/pure.exe"; : > "$PURE"
printf 'KERNEL32.dll\nUSER32.dll\n' > "${IMPORTS}/pure.exe"
DEST2="${TMP}/dest2"; mkdir -p "$DEST2"
OUT2="$(${SH32:-bash} "$S" "$PURE" "$DEST2" 2>&1)"
case "$OUT2" in *"(none needed"*) ok T5 "reports an empty closure" ;;
    *) bad T5 "expected '(none needed', got: $OUT2" ;; esac

rm -rf "$TMP"
if [ "$FAIL" -eq 0 ]; then echo "ALL OK"; else echo "FAILURES"; fi
exit "$FAIL"
