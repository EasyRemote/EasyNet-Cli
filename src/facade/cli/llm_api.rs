// EasyNet CLI — `easynet llm-api` subcommand
// =============================================
//
// File: src/facade/cli/llm_api.rs
// Description: tiny OpenAI-shape chat client. Sends a request
//              through `openai.chat_completions` (RFC-006-C
//              v0.1, device-local OpenAI shim) and prints the
//              assistant reply.
//
//              Goal: silan can do `easynet llm-api "tell me a
//              joke"` from any terminal and the call goes through
//              the same path Cursor / Continue would take.
//
// Defaults:
//   --model    : canonical agent-owned chat Ability URA; defaults to
//                the first model id returned by list_models
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
    /// Canonical agent-owned chat Ability URA (default: first id from list_models).
    #[arg(long, value_name = "ABILITY_URA")]
    pub model: Option<String>,
    /// API key bearer. Default: $EASYNET_API_KEY env, then
    /// '~/.easynet/api_keys.local.toml' (written by
    /// 'easynet api-key create').
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
        crate::runtime::agents::openai_compat_ability::validate_chat_model_id(&m)?;
        return Ok(m);
    }
    // Ask the device-local OpenAI shim what chat-base abilities
    // this host advertises; pick first.
    let result = invoke_local_ability("openai.list_models", json!({}))
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

    let result = invoke_local_ability("openai.chat_completions", adapter_args)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_model_accepts_explicit_agent_chat_ability_ura() {
        let model = "easynet:///r/easynet.run/ability/alice.codex.chat";
        assert_eq!(
            pick_model(Some(model.to_string())).expect("valid model"),
            model
        );
    }

    #[test]
    fn pick_model_rejects_retired_bare_model_name() {
        let err = pick_model(Some("codex".to_string())).expect_err("bare model must fail");
        assert!(
            format!("{err}").contains("canonical Ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pick_model_rejects_retired_local_chat_registry_key() {
        let err =
            pick_model(Some("codex.chat".to_string())).expect_err("local model key must fail");
        assert!(
            format!("{err}").contains("canonical Ability URA"),
            "unexpected error: {err}"
        );
    }
}
