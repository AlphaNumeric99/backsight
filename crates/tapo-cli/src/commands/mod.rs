//! Subcommands of the `tapo` CLI. Each module exposes an argument struct deriving
//! [`clap::Args`] and `pub fn run(args: &Args) -> anyhow::Result<()>`.

pub mod remux;
