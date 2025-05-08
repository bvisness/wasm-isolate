#!/bin/bash

set -euo pipefail

cargo run -- replay $1.wasm -o $1-instrumented.wasm -f $2
wasm-tools dump -o $1-instrumented.dump $1-instrumented.wasm
wasm-tools print -o $1-instrumented.wat $1-instrumented.wasm
wasm-tools validate --features all $1-instrumented.wasm
