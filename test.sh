#!/bin/bash

set -euo pipefail

cargo run $1.wasm -f $2 -o $1-isolated.wasm
wasm-tools print -o $1-isolated.wat $1-isolated.wasm
wasm-tools validate $1-isolated.wasm
