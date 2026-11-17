#!/bin/sh
# Run Test262 (the official JavaScript conformance suite) against the Boa version Ironswift uses.
# Downloads Boa and the Test262 commit that Boa pins into ./conformance (about 400 MB), builds
# Boa's test runner, and prints the totals. Takes 20–40 minutes.
set -e
VERSION=$(grep -A1 'name = "boa_engine"' Cargo.lock | sed -n 's/version = "\(.*\)"/\1/p' | head -1)
TAG="v$(echo "$VERSION" | sed 's/\.0$//')"
mkdir -p conformance && cd conformance
[ -d boa ] || git clone --depth 1 --branch "$TAG" https://github.com/boa-dev/boa boa
cd boa
COMMIT=$(sed -n 's/^commit = "\(.*\)"/\1/p' test262_config.toml)
if [ ! -d test262/.git ]; then
  mkdir -p test262 && cd test262 && git init -q && git remote add origin https://github.com/tc39/test262
  git fetch -q --depth 1 origin "$COMMIT" && git checkout -q FETCH_HEAD && cd ..
fi
cargo build --release -p boa_tester
mkdir -p results
./target/release/boa_tester run -c test262_config.toml -o results | grep -v '^[.F-]*$'
