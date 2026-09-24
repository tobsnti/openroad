#!/usr/bin/env python3
"""Refuse to publish anything that carries private material.

Written after the 2026-09-01 upstream history rewrite, which existed precisely
because five kinds of material had reached a public repository: live server
credentials, an all-rights-reserved font, recovered vendor-internal source
filenames, a transcribed cutscene, and the PK2 archive key. This tree is a
private fork and legitimately holds all of it. The gate is not about what we
may keep -- it is about what may cross the boundary in a patch.

Run it on a diff, not on the tree: `scripts/check_no_private.py <base>...<head>`.
Exit 1 and name the file, line and pattern on any hit.
"""
import re, subprocess, sys

# Any TLD counts. An allowlist of endings is a hole by construction:
# `gw.<shard>.pw` is an endpoint and `.pw` would simply not be on such a list.
# What is excluded instead is
# the one shape that produces the false alarms an allowlist is there to avoid:
# a file name, so `src/net/agent.rs:120` and `0x3013.log` still read as paths.
_FILE_EXT = (r'(?:rs|toml|md|py|json|ya?ml|txt|log|lock|sh|ps1|bat|rc|ron|csv|tsv|html|css|js|ts'
             r'|cs|c|cc|cpp|cxx|h|hh|hpp|java|go|php|rb|lua|swift|kt|ini|cfg|conf|xml'
             r'|png|jpg|jpeg|gif|dds|ddj|wgsl|glb|gltf|bsr|bms|pk2|zip|gz|exe|dll|so|dylib'
             r'|ogg|wav|mp3|db|sql|bak|rlib|wasm)')

PATTERNS = [
    ("pk2 key/salt",      re.compile(r'\bKEY_BYTES\b|\bconst\s+(KEY|SALT)\b|"169841"')),
    ("gateway address",   re.compile(r'\b\d{1,3}(?:\.\d{1,3}){3}\b(?<!127\.0\.0\.1)(?<!0\.0\.0\.0)')),
    # Left `\b` dropped on purpose: an account name is also a leak when it is
    # glued to a prefix (`_` is a word character and would kill the boundary).
    # `\d+`, not `\d\b`: with the boundary, `admin10` reads as `admin1`
    # followed by a non-boundary and the rule stays silent.
    ("test account",      re.compile(r'admin\d+')),
    ("vendor filenames",  re.compile(r'(?i)client-module-map|\bjoymax\b')),
    # Case-insensitive and `/home/` as well: a macOS-only spelling misses a
    # Linux checkout path (`/home/<name>/...`) and `/users/`
    # entirely.
    ("machine path",      re.compile(r'(?i)/(?:users|home)/[a-z0-9._-]+/')),
    # Windows form of the same leak. The POSIX pattern above cannot see it:
    # the separator is a backslash and the drive letter is optional, so a
    # `C:\\Users\\<name>\\...` path would cross the gate silently. A silent hole
    # in the gate is worse than a loud file, so both spellings are checked.
    ("windows machine path",
     re.compile(r'(?i)[a-z]:\\{1,2}users\\{1,2}|(?<![a-z]):?\\{1,2}users\\{1,2}[^\\\s]')),
    ("quarantine path",   re.compile(r'sro-vsro-quarantine')),
    # The address rule above is numeric only, so a live endpoint written as a
    # NAME crosses it untouched (`filter.<host>.net` is an endpoint the numeric
    # rule cannot see). Two rules,
    # because one alone is either blind or unusable:
    #   1. a service-prefixed FQDN — the shape a gateway/agent endpoint has,
    #   2. any FQDN carrying an explicit port — an endpoint, whatever it is called.
    # Exempt is only what cannot name a live host: the RFC 2606 example.*
    # placeholders and localhost. `.invalid` deliberately stays non-exempt, so
    # the tests can prove the rule fires without writing a real host down.
    ("gateway host",
     re.compile(r'(?i)\b(?:filter|gateway|gw|agent|login|shard|div\d*)[a-z0-9-]*'
                r'\.(?!example\.[a-z]{2,}\b)[a-z0-9-]+'
                r'\.(?!' + _FILE_EXT + r'\b)[a-z]{2,24}\b(?!\s*\()')),
    ("host:port endpoint",
     re.compile(r'(?i)(?<![a-z0-9.-])[a-z][a-z0-9-]*(?:\.[a-z0-9-]+)*'
                r'\.(?!' + _FILE_EXT + r'\b)[a-z]{2,24}'
                r'(?<!example\.com)(?<!example\.net)(?<!example\.org)'
                r':\d{2,5}\b')),
]

# A four-level item TID (`3.3.10.1`, RefObjCommon type1..type4) has exactly the
# shape of a dotted quad, so the address rule fired on every alchemy doc line.
# The exemption is deliberately made of TWO conditions that a real address in a
# patch does not meet together, because a narrowing that also lets a real
# address through would be worse than the false alarm it removes:
#   1. the number is a code span, i.e. wrapped in backticks, and
#   2. the same line calls it a TID in words.
# Condition 2 is the load-bearing one: it cannot happen by accident, so pasting
# a server address still trips the gate unless someone writes "TID" next to it,
# and that is a deliberate act a reviewer can see in the diff.
_TID_WORD = re.compile(r'(?i)\bTIDs?\b')

# The client version `1.188.0.0` is also shaped like a dotted quad, and it is
# written down all over this repo (ADR/format docs, the launcher banner), so
# the address rule blocked `make ci` on a version bump. Exempt is only the
# 1.188.x.y family — the version this client clones — not four-part numbers in
# general, so a pasted address cannot hide behind the exemption.
_CLIENT_VERSION = re.compile(r'1\.188\.\d{1,3}\.\d{1,3}')

def _is_client_version(m):
    return bool(_CLIENT_VERSION.fullmatch(m.group(0)))


def _is_item_tid(line, m):
    return bool(_TID_WORD.search(line)) and line[m.start() - 1:m.start()] == "`" \
        and line[m.end():m.end() + 1] == "`"

def scan_line(line):
    """Every pattern that fires on one added line, as `(name, token)` pairs.

    Its own function so the tests measure the rule the gate actually runs.
    """
    out = []
    for name, rx in PATTERNS:
        # finditer, not search: one exempted item TID at the start of a line
        # must not hide a real address later on the same line.
        for m in rx.finditer(line):
            if name == "gateway address" and (_is_item_tid(line, m)
                                              or _is_client_version(m)):
                continue
            out.append((name, m.group(0)[:60]))
            break
    return out


def main(rng):
    # A gate that cannot read its own diff must fail loudly. Without this check
    # a bad range, a missing base or any other git error yields an empty
    # stdout, which the loop below reads as "no hits" and reports as OK --
    # exit 0 on an unscanned patch.
    proc = subprocess.run(["git", "diff", "-U0", rng], capture_output=True, text=True)
    if proc.returncode != 0:
        print(f"check_no_private: git diff {rng} failed ({proc.returncode}); "
              f"nothing was scanned\n{proc.stderr.strip()}")
        return 2
    out = proc.stdout
    path, hits = None, []
    for line in out.splitlines():
        if line.startswith("+++ b/"):
            path = line[6:]
            # The gate's own patterns contain the very strings it refuses, so
            # scanning itself is a guaranteed false positive. Skipping it is
            # not a loophole: a change to this file is a change to the rule,
            # and rule changes are reviewed as rules.
            # ...and its test file for the same reason: the fixtures that
            # prove a rule fires must spell out what the rule refuses.
            if path.endswith(("scripts/check_no_private.py",
                              "scripts/test_check_no_private.py")):
                path = None
            else:
                # The path itself is content too: a dump directory named after
                # a live account leaks it even when every added line is clean.
                for name, tok in scan_line(path):
                    hits.append((path, name, tok, "in the file name"))
        elif path is None:
            continue
        elif line.startswith("+") and not line.startswith("+++"):
            for name, tok in scan_line(line):
                hits.append((path, name, tok, line.strip()[:90]))
    if not hits:
        print(f"check_no_private: OK ({rng})")
        return 0
    print(f"check_no_private: {len(hits)} hit(s) in {rng}\n")
    for p, name, tok, ctx in hits:
        print(f"  {p}\n    {name}: {tok}\n    {ctx}\n")
    print("Nothing in this list may cross into a public repository.")
    return 1

if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "origin/main...HEAD"))
