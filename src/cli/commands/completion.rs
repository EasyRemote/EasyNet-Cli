// EasyNet CLI — Shell Completion
// ==============================
//
// File: src/cli/completion.rs
// Description: `easynet completion <shell>` — emit shell-completion source
//              for bash, zsh, fish, elvish, or PowerShell.
//
// Usage:
//     easynet completion zsh > ~/.zsh/_easynet
//     easynet completion bash > /etc/bash_completion.d/easynet
//
// The implementation defers to clap_complete; the parent `App` derive is
// passed in by main.rs via a thin builder so this module does not need to
// know the full command tree.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, CommandFactory, ValueEnum};
use clap_complete::{generate, Shell};

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: ShellChoice,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ShellChoice {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

impl From<ShellChoice> for Shell {
    fn from(c: ShellChoice) -> Self {
        match c {
            ShellChoice::Bash => Shell::Bash,
            ShellChoice::Zsh => Shell::Zsh,
            ShellChoice::Fish => Shell::Fish,
            ShellChoice::Elvish => Shell::Elvish,
            ShellChoice::Powershell => Shell::PowerShell,
        }
    }
}

pub fn run<C: CommandFactory>(args: CompletionArgs) -> anyhow::Result<()> {
    let mut cmd = C::command();
    let bin_name = cmd.get_name().to_string();
    generate(
        Shell::from(args.shell),
        &mut cmd,
        bin_name,
        &mut std::io::stdout(),
    );
    Ok(())
}
