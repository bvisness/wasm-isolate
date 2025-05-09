#!/bin/bash

set -euo pipefail

echo "Building and instrumenting..."
cargo run -- replay $1.wasm -o $1-instrumented.wasm -f $2

echo "Dumping..."
wasm-tools dump -o $1-instrumented.dump $1-instrumented.wasm

echo "Printing..."
wasm-tools print -o $1-instrumented.wat $1-instrumented.wasm

echo "Validating..."
wasm-tools validate --features all $1-instrumented.wasm
