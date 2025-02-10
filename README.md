# wasm-isolate

A tool to reduce a WebAssembly module to specific features of interest, without breaking validation.

wasm-isolate will walk the module and preserve any other functions, tables, memories, etc. that are required to validate the requested items. It can preserve all features of a WebAssembly module, including types, functions, globals, data segments, etc.

## Installation

[Install Rust](https://www.rust-lang.org/tools/install) or whatever. Then:

```
cargo install --path .
```

## Usage

```
> wasm-isolate --help
wasm-isolate strips a WebAssembly module down to specific features of interest without breaking validation.

Usage: wasm-isolate [OPTIONS] <FILENAME>

Arguments:
  <FILENAME>  The file to read from, or "-" to read from stdin

Options:
      --types <TYPES>...        Type indices to preserve, separated by commas
  -f, --funcs <FUNCS>...        Function indices to preserve, separated by commas
  -t, --tables <TABLES>...      Table indices to preserve, separated by commas
  -g, --globals <GLOBALS>...    Global indices to preserve, separated by commas
  -m, --memories <MEMORIES>...  Memory indices to preserve, separated by commas
  -d, --datas <DATAS>...        Data segment indices to preserve, separated by commas
  -e, --elems <ELEMS>...        Elem segment indices to preserve, separated by commas
      --tags <TAGS>...          Tag indices to preserve, separated by commas
  -o, --out <OUT>
  -h, --help                    Print help
  -V, --version                 Print version
```
