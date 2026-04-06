// EasyNet CLI — Multi-Agent Conversation Orchestrator
// =====================================================
//
// File: src/agent/conversation.rs
// Description: Orchestrates multi-round discussions between registered agents.
//              Each round, each agent sees the topic + prior exchanges as context.
//              Final round optionally asks the first agent to synthesize into an article.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use console::style;
use serde::{Deserialize, Serialize};

use crate::shared::agents::AgentRegistry;

use super::dispatch;

// ─── Config ──────────────────────────────────────────────────────────────────

pub struct ConversationConfig {
    pub agent_names: Vec<String>,
    pub topic: String,
    pub rounds: usize,
    pub max_context_chars: usize,
    pub output_path: Option<String>,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            agent_names: Vec::new(),
            topic: String::new(),
            rounds: 3,
            max_context_chars: 12_000,
            output_path: None,
        }
    }
}

// ─── Log types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    pub agent: String,
    pub prompt_summary: String,
    pub response: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Round {
    pub round_num: usize,
    pub exchanges: Vec<Exchange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationLog {
    pub topic: String,
    pub rounds: Vec<Round>,
    pub final_article: Option<String>,
}

// ─── Orchestration ───────────────────────────────────────────────────────────

pub fn run_conversation(
    registry: &AgentRegistry,
    config: &ConversationConfig,
) -> anyhow::Result<ConversationLog> {
    // Validate all agents exist.
    for name in &config.agent_names {
        if !registry.agents.contains_key(name) {
            anyhow::bail!("agent '{}' not found in registry. Run `easynet agent list` to see registered agents.", name);
        }
    }

    let mut rounds = Vec::new();
    let mut all_exchanges: Vec<Exchange> = Vec::new();

    for round_num in 1..=config.rounds {
        eprintln!("\n{}", style(format!("═══ Round {round_num}/{} ═══", config.rounds)).cyan().bold());

        let mut round_exchanges = Vec::new();
        let is_final_round = round_num == config.rounds;

        for (agent_idx, agent_name) in config.agent_names.iter().enumerate() {
            let entry = registry.agents.get(agent_name).unwrap();

            // Build the prompt for this agent in this round.
            let context = build_context(&all_exchanges, config.max_context_chars);

            let round_instruction = if is_final_round && agent_idx == 0 {
                "This is the FINAL round. Synthesize all perspectives into a cohesive, \
                 insightful article. Write with bold vision and concrete technical arguments."
            } else if is_final_round {
                "This is the FINAL round. Add your concluding thoughts, building on the synthesis above."
            } else if round_num == 1 {
                "This is the opening round. Present your initial perspective with depth and specificity."
            } else {
                "Build on the discussion above. Challenge, extend, or synthesize the ideas presented. \
                 Add new angles and concrete technical insights."
            };

            let prompt = format!(
                "## Topic\n\n{topic}\n\n## Instructions\n\nYou are {agent_name}. {instruction}\n",
                topic = config.topic,
                instruction = round_instruction,
            );

            eprint!(
                "  {} {} ... ",
                style(format!("[{agent_name}]")).yellow().bold(),
                if is_final_round && agent_idx == 0 { "synthesizing" } else { "thinking" }
            );

            let response = dispatch::send_to_agent(
                agent_name,
                entry,
                &prompt,
                if context.is_empty() { None } else { Some(&context) },
                None,
                None,
            )?;

            eprintln!(
                "{} ({}s)",
                style("done").green(),
                response.duration_ms / 1000
            );

            // Print a preview of the response.
            let preview: String = response.content.chars().take(200).collect();
            eprintln!("  {} {}{}", style("│").dim(), preview.trim(), if response.content.len() > 200 { "..." } else { "" });

            let exchange = Exchange {
                agent: agent_name.clone(),
                prompt_summary: format!("Round {round_num}"),
                response: response.content,
                duration_ms: response.duration_ms,
            };
            round_exchanges.push(exchange.clone());
            all_exchanges.push(exchange);
        }

        rounds.push(Round {
            round_num,
            exchanges: round_exchanges,
        });
    }

    // The final article is the last exchange from the final round's first agent.
    let final_article = rounds.last()
        .and_then(|r| r.exchanges.first())
        .map(|e| e.response.clone());

    let log = ConversationLog {
        topic: config.topic.clone(),
        rounds,
        final_article,
    };

    // Write output if path specified.
    if let Some(path) = &config.output_path {
        let markdown = format_as_markdown(&log);
        std::fs::write(path, &markdown)?;
        eprintln!("\n{} Article written to {}", style("✓").green(), style(path).cyan());
    }

    Ok(log)
}

fn build_context(exchanges: &[Exchange], max_chars: usize) -> String {
    if exchanges.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    let mut total_chars = 0;

    // Include exchanges from newest to oldest, truncating oldest if needed.
    for ex in exchanges.iter().rev() {
        let entry = format!("### {} ({}):\n{}\n", ex.agent, ex.prompt_summary, ex.response);
        if total_chars + entry.len() > max_chars && !parts.is_empty() {
            parts.push("[Earlier discussion truncated for brevity]".to_string());
            break;
        }
        total_chars += entry.len();
        parts.push(entry);
    }

    parts.reverse();
    parts.join("\n")
}

fn format_as_markdown(log: &ConversationLog) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {}\n\n", log.topic));
    md.push_str("*Generated by EasyNet multi-agent discussion*\n\n---\n\n");

    if let Some(article) = &log.final_article {
        md.push_str(article);
        md.push_str("\n\n---\n\n");
    }

    md.push_str("## Discussion Log\n\n");
    for round in &log.rounds {
        md.push_str(&format!("### Round {}\n\n", round.round_num));
        for ex in &round.exchanges {
            md.push_str(&format!("**{}** ({}s):\n\n", ex.agent, ex.duration_ms / 1000));
            md.push_str(&ex.response);
            md.push_str("\n\n---\n\n");
        }
    }

    md
}
