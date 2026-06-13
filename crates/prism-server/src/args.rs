//! Hand-rolled argument parsing for the `prism-serve` binary (no external
//! crates).
//!
//! Flags: `--workspace PATH` (or `-w`), `--port N` (or `-p`), `--help`/`-h`.
//! Values may be `--name value` or `--name=value`. Unset values fall back to
//! environment variables, then defaults. A bad invocation yields an
//! [`ArgsError`] the binary prints before exiting non-zero.

use std::fmt;

/// Default workspace path when neither flag nor env var is set.
pub const DEFAULT_WORKSPACE: &str = ".";
/// Default port (architecture doc's local service default).
pub const DEFAULT_PORT: u16 = 7777;
/// Environment variable for the workspace path.
pub const ENV_WORKSPACE: &str = "PRISM_WORKSPACE";
/// Environment variable for the port.
pub const ENV_PORT: &str = "PRISM_PORT";

/// A parsed invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Args {
    /// Run the server with these settings.
    Serve {
        /// Workspace root to open.
        workspace: String,
        /// TCP port (0 = ephemeral).
        port: u16,
    },
    /// Print usage and exit successfully.
    Help,
}

/// Why an invocation could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgsError {
    /// A flag that needs a value had none.
    MissingValue(String),
    /// `--port` was not a valid `u16`.
    BadPort(String),
    /// An unrecognized argument.
    Unknown(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgsError::MissingValue(flag) => write!(f, "flag {flag} requires a value"),
            ArgsError::BadPort(v) => write!(f, "invalid --port value {v:?} (expected 0..=65535)"),
            ArgsError::Unknown(a) => write!(f, "unknown argument {a:?}"),
        }
    }
}

/// The usage string.
pub const USAGE: &str = "\
prism-serve — a hermetic local web UI over a Prism workspace

USAGE:
    prism-serve [--workspace PATH] [--port N]

OPTIONS:
    -w, --workspace PATH   Workspace root to open (default: \".\", env PRISM_WORKSPACE)
    -p, --port N           TCP port to bind on 127.0.0.1 (default: 7777, env PRISM_PORT;
                           0 picks an ephemeral port)
    -h, --help             Print this help and exit
";

/// Parse `argv` (without the program name), reading env defaults via `env`.
///
/// `env(name)` returns the value of environment variable `name`, if set. Passed
/// in so it can be stubbed in tests.
pub fn parse<F>(argv: &[String], env: F) -> Result<Args, ArgsError>
where
    F: Fn(&str) -> Option<String>,
{
    let mut workspace: Option<String> = None;
    let mut port: Option<u16> = None;

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        // Support `--flag=value` by splitting on the first `=`.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with('-') => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };

        match name {
            "-h" | "--help" => return Ok(Args::Help),
            "-w" | "--workspace" => {
                let v = take_value(name, inline, argv, &mut i)?;
                workspace = Some(v);
            }
            "-p" | "--port" => {
                let v = take_value(name, inline, argv, &mut i)?;
                let p = v.parse::<u16>().map_err(|_| ArgsError::BadPort(v.clone()))?;
                port = Some(p);
            }
            other => return Err(ArgsError::Unknown(other.to_string())),
        }
        i += 1;
    }

    let workspace = workspace
        .or_else(|| env(ENV_WORKSPACE))
        .unwrap_or_else(|| DEFAULT_WORKSPACE.to_string());
    let port = match port {
        Some(p) => p,
        None => match env(ENV_PORT) {
            Some(v) => v.parse::<u16>().map_err(|_| ArgsError::BadPort(v))?,
            None => DEFAULT_PORT,
        },
    };

    Ok(Args::Serve { workspace, port })
}

/// Resolve a flag's value: the inline `=value`, else the next argv token (which
/// it consumes by advancing `i`).
fn take_value(
    name: &str,
    inline: Option<String>,
    argv: &[String],
    i: &mut usize,
) -> Result<String, ArgsError> {
    if let Some(v) = inline {
        return Ok(v);
    }
    let next = argv
        .get(*i + 1)
        .ok_or_else(|| ArgsError::MissingValue(name.to_string()))?;
    *i += 1;
    Ok(next.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_when_empty() {
        let a = parse(&[], no_env).unwrap();
        assert_eq!(
            a,
            Args::Serve {
                workspace: ".".into(),
                port: 7777
            }
        );
    }

    #[test]
    fn space_separated_flags() {
        let a = parse(&args(&["--workspace", "/tmp/ws", "--port", "9000"]), no_env).unwrap();
        assert_eq!(
            a,
            Args::Serve {
                workspace: "/tmp/ws".into(),
                port: 9000
            }
        );
    }

    #[test]
    fn equals_form_and_short_flags() {
        let a = parse(&args(&["-w=/a/b", "-p=8080"]), no_env).unwrap();
        assert_eq!(
            a,
            Args::Serve {
                workspace: "/a/b".into(),
                port: 8080
            }
        );
    }

    #[test]
    fn help_short_circuits() {
        assert_eq!(parse(&args(&["--help"]), no_env).unwrap(), Args::Help);
        assert_eq!(
            parse(&args(&["-w", "x", "-h"]), no_env).unwrap(),
            Args::Help
        );
    }

    #[test]
    fn env_fallbacks() {
        let env = |k: &str| match k {
            ENV_WORKSPACE => Some("/env/ws".to_string()),
            ENV_PORT => Some("5555".to_string()),
            _ => None,
        };
        let a = parse(&[], env).unwrap();
        assert_eq!(
            a,
            Args::Serve {
                workspace: "/env/ws".into(),
                port: 5555
            }
        );
    }

    #[test]
    fn explicit_flag_beats_env() {
        let env = |k: &str| (k == ENV_PORT).then(|| "5555".to_string());
        let a = parse(&args(&["--port", "1234"]), env).unwrap();
        if let Args::Serve { port, .. } = a {
            assert_eq!(port, 1234);
        } else {
            panic!("expected serve");
        }
    }

    #[test]
    fn missing_value_errors() {
        assert_eq!(
            parse(&args(&["--port"]), no_env).unwrap_err(),
            ArgsError::MissingValue("--port".into())
        );
    }

    #[test]
    fn bad_port_errors() {
        assert!(matches!(
            parse(&args(&["--port", "99999"]), no_env).unwrap_err(),
            ArgsError::BadPort(_)
        ));
        assert!(matches!(
            parse(&args(&["--port", "abc"]), no_env).unwrap_err(),
            ArgsError::BadPort(_)
        ));
    }

    #[test]
    fn bad_env_port_errors() {
        let env = |k: &str| (k == ENV_PORT).then(|| "nope".to_string());
        assert!(matches!(parse(&[], env).unwrap_err(), ArgsError::BadPort(_)));
    }

    #[test]
    fn unknown_flag_errors() {
        assert_eq!(
            parse(&args(&["--frob"]), no_env).unwrap_err(),
            ArgsError::Unknown("--frob".into())
        );
    }

    #[test]
    fn port_zero_is_valid_ephemeral() {
        let a = parse(&args(&["--port", "0"]), no_env).unwrap();
        if let Args::Serve { port, .. } = a {
            assert_eq!(port, 0);
        } else {
            panic!("expected serve");
        }
    }
}
