// EasyNet CLI — `easynet llm-api` subcommand
// =============================================
//
// File: src/facade/cli/llm_api.rs
// Description: tiny OpenAI-shape chat client. Sends a request
//              through `01HUB.openai.chat_completions` (RFC-006-C
//              v0.1) and prints the assistant reply.
//
//              Goal: silan can do `easynet llm-api "tell me a
//              joke"` from any terminal and the call goes through
//              the same path Cursor / Continue would take.
//
// Defaults:
//   --model    : first chat-base ability the daemon registers
//                (resolved at call time via list_models)
//   --key      : EASYNET_API_KEY env, else
//                ~/.easynet/api_keys.local.toml (written by
//                `easynet api-key create` on success)
//   --system   : optional
//   --json     : emit full chat.completion JSON, else just text
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use serde_json::{json, Value};

use crate::runtime::agents::api_key_ability;
use crate::support::local_invoke::invoke_local_ability;

#[derive(Debug, Args)]
pub struct LlmApiArgs {
    /// User prompt — required positional.
    pub prompt: String,
    /// Model name (default: first chat-base ability on the
    /// daemon, resolved via list_models).
    #[arg(long)]
    pub model: Option<String>,
    /// API key bearer. Default: $EASYNET_API_KEY env, then
    /// `~/.easynet/api_keys.local.toml` (written by
    /// `easynet api-key create`).
    #[arg(long)]
    pub key: Option<String>,
    /// Optional system message.
    #[arg(long)]
    pub system: Option<String>,
    /// Emit full chat.completion JSON instead of just the
    /// assistant text.
    #[arg(long)]
    pub json: bool,
}

fn pick_token(arg: Option<String>) -> Option<String> {
    if let Some(t) = arg {
        return Some(t);
    }
    if let Ok(t) = std::env::var("EASYNET_API_KEY") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    api_key_ability::read_local_default_token()
}

fn pick_model(arg: Option<String>) -> anyhow::Result<String> {
    if let Some(m) = arg {
        return Ok(m);
    }
    // Ask the hub adapter what's available; pick first.
    let result = invoke_local_ability("01HUB.openai.list_models", json!({}))
        .map_err(|e| anyhow::anyhow!("could not list models: {e}"))?;
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("list_models returned no data array"))?;
    if data.is_empty() {
        anyhow::bail!(
            "no chat-base abilities available. \
             Add an agent first: `easynet agent add <name> --type claude-code`"
        );
    }
    let id = data[0]
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("first model has no id"))?;
    Ok(id.to_string())
}

pub fn run(args: LlmApiArgs) -> anyhow::Result<()> {
    let model = pick_model(args.model)?;
    let token = pick_token(args.key);

    // Build OpenAI-shape request.
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = args.system {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    messages.push(json!({ "role": "user", "content": args.prompt }));

    let request = json!({
        "model":    model,
        "messages": messages,
        "stream":   false,
    });

    let mut adapter_args = json!({ "request": request });
    if let Some(t) = token {
        adapter_args["auth_token"] = json!(t);
    }

    eprintln!("[llm-api] model={model}");

    let result = invoke_local_ability("01HUB.openai.chat_completions", adapter_args)
        .map_err(|e| anyhow::anyhow!("chat_completions failed: {e}"))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    // Default: print just the assistant text.
    let text = result
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("(no content)");
    println!("{text}");
    Ok(())
}
