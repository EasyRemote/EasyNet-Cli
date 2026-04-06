// EasyNet CLI — Shell Completion
// ==============================
//
// File: src/cli/completion.rs
// Description: `easynet completion <shell>` — emit a static completion
//              script for bash / zsh / fish / powershell / elvish via
//              `clap_complete`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, CommandFactory};
use clap_complete::{generate, Shell};

use crate::App;

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: Shell,
}

pub fn run(args: CompletionArgs) -> anyhow::Result<()> {
    let mut cmd = App::command();
    let bin_name = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}
