//! The `prism-serve` binary: parse args, open the workspace, start the server,
//! and print the URL. Then block until interrupted (the server runs on
//! background threads, so we park the main thread).

use std::process::ExitCode;

use prism_server::args::{self, Args};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&argv, |k| std::env::var(k).ok()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("prism-serve: {e}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };

    let (workspace, port) = match parsed {
        Args::Help => {
            print!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Args::Serve { workspace, port } => (workspace, port),
    };

    let server = match prism_server::serve(&workspace, port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prism-serve: cannot serve workspace {workspace:?}: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("prism-serve: serving {workspace:?} at {}", server.url());
    println!("prism-serve: press Ctrl-C to stop");

    // The server lives on background threads; park the main thread so dropping
    // `server` (and its shutdown) only happens on process exit.
    loop {
        std::thread::park();
    }
}
