//! `tapo` — a small CLI over the `tapo-camera` crate, used to verify the protocol port
//! against real cameras and the mock camera.
//!
//! Each subcommand lives in its own module under `commands/` and exposes an argument
//! struct (a clap [`Args`](clap::Args)) and a `run` function. To add one, add a variant to
//! [`Command`] and a match arm in [`main`].
//!
//! Logging goes to stderr and is controlled with `RUST_LOG` (default `warn`), for
//! example `RUST_LOG=tapo_camera=debug tapo remux in.mpegts out.mp4`.

mod commands;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "tapo", version, about = "Tools for TP-Link Tapo cameras", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The subcommands, one module each under `commands/`.
#[derive(Debug, Subcommand)]
enum Command {
    /// Convert an MPEG-TS stream or recording from a camera into an MP4 file.
    Remux(commands::remux::RemuxArgs),
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Remux(args) => commands::remux::run(&args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_remux() {
        let cli = Cli::try_parse_from([
            "tapo",
            "remux",
            "in.mpegts",
            "out.mp4",
            "--audio-rate",
            "16000",
        ])
        .unwrap();
        match cli.command {
            Command::Remux(args) => {
                assert_eq!(args.input.to_str(), Some("in.mpegts"));
                assert_eq!(args.output.to_str(), Some("out.mp4"));
                assert_eq!(args.audio_rate, 16_000);
            }
            #[allow(unreachable_patterns)]
            _ => panic!("expected the remux command"),
        }
        assert!(Cli::try_parse_from(["tapo", "remux", "in.mpegts"]).is_err());
    }
}
