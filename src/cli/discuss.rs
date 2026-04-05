// EasyNet CLI — Discuss Subcommand
// ==================================
//
// File: src/cli/discuss.rs
// Description: `easynet discuss` — orchestrate multi-agent discussions.
//
// Usage:
//   easynet discuss --agents claude,codex --rounds 3 --topic "..." [--output <path>]
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;

use crate::agent::conversation::{self, ConversationConfig};
use crate::shared::agents;

#[derive(Debug, Args)]
pub struct DiscussArgs {
    /// Comma-separated list of agent names (e.g. "claude,codex")
    #[arg(long)]
    pub agents: String,

    /// Discussion topic / prompt
    #[arg(long)]
    pub topic: String,

    /// Number of discussion rounds
    #[arg(long, default_value_t = 3)]
    pub rounds: usize,

    /// Max context chars passed to each agent per turn (truncates older rounds)
    #[arg(long, default_value_t = 12_000)]
    pub max_context: usize,

    /// Write the final article to a markdown file
    #[arg(long)]
    pub output: Option<String>,
}

pub fn run(args: DiscussArgs) -> anyhow::Result<()> {
    let agent_names: Vec<String> = args.agents.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    anyhow::ensure!(!agent_names.is_empty(), "no agents specified");
    anyhow::ensure!(args.rounds > 0, "rounds must be >= 1");

    let registry = agents::load_agents()?;

    eprintln!("{}", style("EasyNet Multi-Agent Discussion").cyan().bold());
    eprintln!("{}", style("═".repeat(40)).dim());
    eprintln!("  Agents: {}", agent_names.join(", "));
    eprintln!("  Rounds: {}", args.rounds);
    eprintln!("  Topic:  {}", &args.topic[..args.topic.len().min(80)]);
    if args.topic.len() > 80 {
        eprintln!("          ...");
    }

    let config = ConversationConfig {
        agent_names,
        topic: args.topic,
        rounds: args.rounds,
        max_context_chars: args.max_context,
        output_path: args.output,
    };

    let log = conversation::run_conversation(&registry, &config)?;

    eprintln!("\n{}", style("═".repeat(40)).dim());
    eprintln!(
        "{} Discussion complete — {} rounds, {} exchanges",
        style("✓").green(),
        log.rounds.len(),
        log.rounds.iter().map(|r| r.exchanges.len()).sum::<usize>(),
    );

    Ok(())
}
