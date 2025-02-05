use std::io::Read;

use anyhow::{Ok, Result};
use clap::Parser as _;
use wasmparser::{Parser, Payload::*};

/// Simple program to greet a person
#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    filename: String,
}

fn main() {
    let cli = Args::parse();

    let filename = cli.filename;
    println!("Value for name: {filename}");
}

fn parse(mut reader: impl Read) -> Result<()> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let parser = Parser::new(0);

    for payload in parser.parse_all(&buf) {
        match payload? {
            // Sections for WebAssembly modules
            Version { .. } => { /* ... */ }
            TypeSection(_) => { /* ... */ }
            ImportSection(_) => { /* ... */ }
            FunctionSection(_) => { /* ... */ }
            TableSection(_) => { /* ... */ }
            MemorySection(_) => { /* ... */ }
            TagSection(_) => { /* ... */ }
            GlobalSection(_) => { /* ... */ }
            ExportSection(_) => { /* ... */ }
            StartSection { .. } => { /* ... */ }
            ElementSection(_) => { /* ... */ }
            DataCountSection { .. } => { /* ... */ }
            DataSection(_) => { /* ... */ }

            // Here we know how many functions we'll be receiving as
            // `CodeSectionEntry`, so we can prepare for that, and
            // afterwards we can parse and handle each function
            // individually.
            CodeSectionStart { .. } => { /* ... */ }
            CodeSectionEntry(body) => {
                // here we can iterate over `body` to parse the function
                // and its locals
            }

            // most likely you'd return an error here, but if you want
            // you can also inspect the raw contents of unknown sections
            other => {
                match other.as_section() {
                    Some((id, range)) => { /* ... */ }
                    None => { /* ... */ }
                }
            }
        }
    }

    Ok(())
}
