use anyhow::Result;
use clap::CommandFactory;
use clap_complete::{generate, Shell};
use std::io;

/// Generate shell completion script for the specified shell
pub fn run(shell: Shell) -> Result<()> {
    let mut cmd = crate::Cli::command();
    let bin_name = cmd.get_name().to_string();

    generate(shell, &mut cmd, bin_name, &mut io::stdout());

    Ok(())
}
