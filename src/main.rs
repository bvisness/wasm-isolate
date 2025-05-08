mod isolate;
mod relocation;
mod replay;
mod uses;
mod util;

use anyhow::Result;
use clap::Parser as _;

use isolate::*;
use replay::*;

#[derive(clap::Parser, Debug)]
#[clap(args_conflicts_with_subcommands = true)]
#[command(
    version,
    about = "wasm-isolate strips a WebAssembly module down to specific features of interest without breaking validation."
)]
struct Args {
    #[clap(subcommand)]
    sub: Option<Commands>,
    // #[clap(flatten)]
    // isolate: IsolateArgs,
}

#[derive(clap::Parser, Debug)]
enum Commands {
    /// Strip a WebAssembly module down to specific features of interest. The default command.
    Isolate(IsolateArgs),

    /// Instrument a WebAssembly module to record and replay runs of a specific function.
    Replay(ReplayArgs),
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.sub {
        Some(Commands::Isolate(args)) => isolate(args),
        Some(Commands::Replay(args)) => replay(args),
        // None => isolate(args.isolate),
        None => panic!("no!"),
    }
}
