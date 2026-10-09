//! Command-line surface, parsed with `clap`'s derive API.
//!
//! The default invocation (no arguments) launches the GUI; the rest is a
//! deliberately small `rea`-style command set that works headless.

use clap::{Parser, Subcommand};

/// Termius — the native Rust port of the Termius desktop SSH client.
#[derive(Debug, Parser)]
#[command(name = "termius", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands — kept minimal; the GUI is the default (no subcommand).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report which engines, paths and helpers this installation can use.
    Doctor,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_invocation_has_no_subcommand() {
        let cli = Cli::try_parse_from(["termius"]).expect("parse default invocation");
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_the_doctor_subcommand() {
        let cli = Cli::try_parse_from(["termius", "doctor"]).expect("parse doctor");
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }

    #[test]
    fn help_and_version_are_built_in() {
        use clap::error::ErrorKind;

        let version = Cli::try_parse_from(["termius", "--version"])
            .expect_err("--version exits through the parser");
        assert!(matches!(version.kind(), ErrorKind::DisplayVersion));

        let help = Cli::try_parse_from(["termius", "--help"])
            .expect_err("--help exits through the parser");
        assert!(matches!(help.kind(), ErrorKind::DisplayHelp));
    }
}
