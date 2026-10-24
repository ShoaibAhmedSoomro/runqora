# shared helpers for the stack build scripts (bash, Linux and macOS)
set -euo pipefail
NPROC=$( (nproc 2>/dev/null || sysctl -n hw.ncpu) )
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
# fetch URL FILE [SHA256]: download and check; an empty hash only prints the one found so it can be pinned
fetch() {
  curl -fsSL --retry 5 -o "$2" "$1"
  local got; got=$(sha256 "$2")
  if [ -n "${3:-}" ] && [ "$got" != "$3" ]; then echo "hash mismatch for $1: $got" >&2; exit 1; fi
  [ -n "${3:-}" ] || echo "UNPINNED $1 sha256 $got"
}
