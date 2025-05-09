#!/bin/bash

set -euo pipefail

echo "Building and instrumenting..."
cargo run -- replay $1.wasm -o $1 -f $2

echo "Dumping..."
wasm-tools dump -o $1/record.dump $1/record.wasm
wasm-tools dump -o $1/replay.dump $1/replay.wasm

echo "Printing..."
wasm-tools print -o $1/record.wat $1/record.wasm
wasm-tools print -o $1/replay.wat $1/replay.wasm

echo "Validating..."
wasm-tools validate --features all $1/record.wasm
wasm-tools validate --features all $1/replay.wasm
