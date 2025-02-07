#!/bin/bash

set -euo pipefail

wasm-tools print -o $1.wat $1.wasm
cargo run $1.wasm -f $2 -o $1-isolated.wasm
wasm-tools print -o $1-isolated.wat $1-isolated.wasm
wasm-tools validate --features all $1-isolated.wasm
