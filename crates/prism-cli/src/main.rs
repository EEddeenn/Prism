//! Prism CLI entry point: parse `argv`, dispatch via [`prism_cli::run`], and
//! exit with the stable code it returns.

use std::io::Write;

fn main() {
    let args = std::env::args().skip(1);
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    let code = prism_cli::run(args, &mut out, &mut err);
    // Flush before exiting; ignore errors (a broken pipe is not actionable here).
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
