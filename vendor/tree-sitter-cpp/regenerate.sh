#!/bin/sh
# Maintenance only. Cargo builds from the checked-in gzip and C sources.
# From the project root: sh vendor/tree-sitter-cpp/regenerate.sh /path/to/tree-sitter
set -eu

# tree-sitter-cpp 0.23.4 (crates.io .cargo_vcs_info.json), and the tree-sitter-c
# and CLI versions from that revision's package-lock.json. This emits ABI 14.
cpp_rev=f41e1a044c8a84ea9fa8577fdd2eab92ec96de02
c_version=0.23.1
cli_version='tree-sitter 0.24.3 (bdfe32402e85673bbc693216f0a6ef72c98bb665)'
cli=${1:-tree-sitter}
cli=$(command -v "$cli")
case "$cli" in /*) ;; *) cli="$PWD/$cli" ;; esac
if [ "$("$cli" --version)" != "$cli_version" ]; then
    printf 'Expected %s\n' "$cli_version" >&2
    exit 1
fi

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
curl -fsSL "https://codeload.github.com/tree-sitter/tree-sitter-cpp/tar.gz/$cpp_rev" -o cpp.tgz
curl -fsSL "https://registry.npmjs.org/tree-sitter-c/-/tree-sitter-c-$c_version.tgz" -o c.tgz
sha256sum -c <<'CHECKSUMS'
ad4b9b56602e9b14082637988f0042521006819960ed8c308bc24ded9ceb9331  cpp.tgz
460649a4a46cd14115428ea0a00828eefc951ffb34fa5c3622effed97f4bdbef  c.tgz
CHECKSUMS
mkdir -p node_modules/tree-sitter-c
tar -xzf cpp.tgz --strip-components=1
tar -xzf c.tgz --strip-components=1 -C node_modules/tree-sitter-c
patch -F 0 -p1 < "$here/grammar.patch"
"$cli" generate --abi 14
gzip -9nc src/parser.c > parser.c.gz
mkdir -p "$here/src/tree_sitter"
cp parser.c.gz src/scanner.c "$here/src/"
cp src/tree_sitter/alloc.h src/tree_sitter/parser.h "$here/src/tree_sitter/"
cp LICENSE "$here/"
