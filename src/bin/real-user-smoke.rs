// EasyNet CLI — real-user smoke reproducer
// =========================================
//
// Run a real-disk / real-binary / real-API exercise of the
// load-bearing abilities (fs.write, fs.read, process.exec,
// shell.run, <agent>.chat) end-to-end through the actual
// dispatcher, and print what each one returned. Distinct from
// the unit-test layer in two ways:
//
//   1. No tempdir trickery for the input — fs.write into
//      `target/real-user-scratch/`, process.exec on /bin/cat
//      against /etc/hosts, shell.run with a real `git
//      rev-parse --short HEAD && uname -s` against this repo.
//      The point is "user pulls the repo, runs this, sees the
//      same bytes their `cat` would show."
//
//   2. The chat section optionally hits the real Claude /
//      Codex CLI when EASYNET_REAL_CHAT_OK=1 is set. That call
//      DOES cost API credits, so it's gated explicitly.
//
// Use this binary as a manual reproducer when you want to
// confirm the end-to-end paths work on a fresh checkout. The
// assertions also live as unit tests in
// runtime::agents::real_invoke_tests for the deterministic
// CI-runnable subset.
//
//   cargo run --bin real-user-smoke
//   EASYNET_REAL_CHAT_OK=1 cargo run --bin real-user-smoke
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::Arc;

use easynet_cli::runtime::ability_dispatch::AbilityDispatcher;
use easynet_cli::runtime::agents::{build_registry, build_registry_for_daemon};
use easynet_cli::runtime::gateway::NoopGateway;
use easynet_cli::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

fn target(ability: &str, args: Value) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: ability.to_string(),
        normalized_args: args,
        call_mode: CallMode::Rpc,
    }
}

fn d() -> AbilityDispatcher {
    AbilityDispatcher::new(build_registry(), Arc::new(NoopGateway::new()))
}

fn b64(s: &str) -> String {
    let bytes = BASE64_STANDARD.decode(s).unwrap();
    String::from_utf8(bytes).unwrap_or_else(|e| format!("<non-utf8: {e}>"))
}

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    println!("=== fs.write ===");
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scratch = manifest.join("target").join("real-user-scratch");
    std::fs::create_dir_all(&scratch)?;
    let out = scratch.join("greeting.txt");
    let _ = std::fs::remove_file(&out);

    let resp = d().execute_rpc(target(
        "fs.write",
        json!({
            "path": out.to_str().unwrap(),
            "content": "Hello from a real fs.write call.\nLine 2 of the file.\n",
            "encoding": "utf8",
        }),
    ))?;
    println!("Response: {}", serde_json::to_string_pretty(&resp)?);
    let on_disk = std::fs::read_to_string(&out)?;
    println!("On disk after fs.write:\n---\n{on_disk}---");
    let len = on_disk.len();
    println!("Length on disk: {len}");

    // fs.read it back to confirm round-trip
    println!("\n=== fs.read of the file we just wrote ===");
    let read_resp = d().execute_rpc(target(
        "fs.read",
        json!({"path": out.to_str().unwrap(), "encoding": "utf8"}),
    ))?;
    println!(
        "fs.read content: {:?}",
        read_resp["content"].as_str().unwrap()
    );
    println!("fs.read size: {}", read_resp["size"]);

    println!("\n=== process.exec /bin/cat /etc/hosts ===");
    let etc_hosts = "/etc/hosts";
    if !std::path::Path::new(etc_hosts).exists() {
        println!("(skip: /etc/hosts not present on this host)");
    } else {
        let resp = rt.block_on(async {
            tokio::task::spawn_blocking(|| {
                d().execute_rpc(target(
                    "process.exec",
                    json!({"command": "/bin/cat", "args": ["/etc/hosts"]}),
                ))
            })
            .await
            .unwrap()
        })?;
        println!("ok: {}", resp["ok"]);
        println!("exit_code: {}", resp["exit_code"]);
        println!("stdout_bytes: {}", resp["stdout_bytes"]);
        let stdout = b64(resp["stdout"].as_str().unwrap());
        println!("First 8 lines of /etc/hosts via process.exec:");
        for (i, line) in stdout.lines().take(8).enumerate() {
            println!("  {}: {line}", i + 1);
        }
        println!("(snip; total {} chars)", stdout.len());
    }

    println!("\n=== shell.run 'git rev-parse --short HEAD && uname -s' ===");
    let resp = rt.block_on(async {
        tokio::task::spawn_blocking(|| {
            d().execute_rpc(target(
                "shell.run",
                json!({
                    "command": "git rev-parse --short HEAD && uname -s",
                    "cwd": env!("CARGO_MANIFEST_DIR"),
                }),
            ))
        })
        .await
        .unwrap()
    });
    match resp {
        Ok(r) => {
            println!("ok: {}", r["ok"]);
            println!("exit_code: {}", r["exit_code"]);
            let stdout = b64(r["stdout"].as_str().unwrap());
            println!("stdout:\n---\n{stdout}---");
            let stderr = b64(r["stderr"].as_str().unwrap());
            if !stderr.is_empty() {
                println!("stderr:\n---\n{stderr}---");
            }
        }
        Err(e) => println!("shell.run failed: {e}"),
    }

    println!("\n=== shell.run with permission rules: 'ls -la /tmp' ===");
    let resp = rt.block_on(async {
        tokio::task::spawn_blocking(|| {
            d().execute_rpc(target(
                "shell.run",
                json!({
                    "command": "ls -la /tmp",
                    "allow_rules": [{"argv0": "ls"}],
                }),
            ))
        })
        .await
        .unwrap()
    });
    match resp {
        Ok(r) => {
            println!("ok: {}", r["ok"]);
            if r["ok"] == json!(true) {
                let stdout = b64(r["stdout"].as_str().unwrap());
                let preview: String = stdout.lines().take(5).collect::<Vec<_>>().join("\n");
                println!("first 5 lines of `ls -la /tmp`:\n{preview}");
            } else {
                println!("rejection: {}", serde_json::to_string_pretty(&r)?);
            }
        }
        Err(e) => println!("shell.run failed: {e}"),
    }

    println!("\n=== shell.run REJECTION: 'rm -rf /tmp/x' without destructive ack ===");
    let resp = rt.block_on(async {
        tokio::task::spawn_blocking(|| {
            d().execute_rpc(target(
                "shell.run",
                json!({"command": "rm /tmp/x"}),
            ))
        })
        .await
        .unwrap()
    });
    match resp {
        Ok(r) => {
            println!("response: {}", serde_json::to_string_pretty(&r)?);
        }
        Err(e) => println!("shell.run errored: {e}"),
    }

    println!("\n=== <agent>.chat real invocation ===");
    if std::env::var("EASYNET_REAL_CHAT_OK").is_err() {
        println!("(skipped: set EASYNET_REAL_CHAT_OK=1 to run a real chat call");
        println!(" against the locally-installed claude/codex CLI. WARNING: this");
        println!(" hits the Anthropic / OpenAI API and costs money.)");
    } else {
        // 1. Spin up a mission run dir so the dispatch invariant is
        //    satisfied. Mirrors what
        //    facade::cli::mission_runs::root_dir() returns:
        //    `$HOME/.easynet/missions/runs/`.
        let mission_id = format!("smoke-{}", std::process::id());
        let home = std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let mission_root = home.join(".easynet").join("missions").join("runs");
        let mission_dir = mission_root.join(&mission_id);
        std::fs::create_dir_all(&mission_dir)?;
        std::env::set_var("EASYNET_MISSION_ID", &mission_id);

        // 2. Build registry from the developer's ACTUAL ~/.easynet/agents.json
        //    so existing `claude` / `codex` rows are picked up. This bin is
        //    deliberately running against the developer's real config — the
        //    EASYNET_REAL_CHAT_OK guard signals consent.
        let reg = build_registry_for_daemon(
            Arc::new(easynet_cli::runtime::execution::session::SessionService::new()),
            Arc::new(easynet_cli::runtime::execution::permission::PermissionService::new()),
            Arc::new(easynet_cli::runtime::execution::discuss::DiscussService::new()),
            Arc::new(easynet_cli::runtime::execution::schedule::ScheduleService::new()),
            Arc::new(easynet_cli::runtime::execution::loop_instance::LoopService::new()),
            Arc::new(Vec::new()),
        );
        let advertised = reg.list_abilities();
        let chat_ability = advertised
            .iter()
            .find(|n| n.ends_with(".chat"));
        let chat_ability = match chat_ability {
            Some(n) => n.clone(),
            None => {
                println!("(skipped: no <agent>.chat registered. Add one with");
                println!(" `easynet agent add <name> --type claude-code` first.)");
                return Ok(());
            }
        };
        println!("Will invoke: {chat_ability}");
        let dispatcher = AbilityDispatcher::new(reg, Arc::new(NoopGateway::new()));

        let prompt = "Reply with EXACTLY the three-word phrase: hello easynet smoke. \
                      No other words. No punctuation. No explanation.";
        println!("Prompt: {prompt:?}");
        println!("Calling {chat_ability} (this will spawn the real CLI)...");

        let chat_for_invoke = chat_ability.clone();
        let result = rt.block_on(async move {
            tokio::task::spawn_blocking(move || {
                dispatcher.execute_rpc(target(
                    &chat_for_invoke,
                    json!({"prompt": prompt, "stream": false}),
                ))
            })
            .await
            .unwrap()
        });

        match result {
            Ok(v) => {
                println!("response keys: {:?}",
                    v.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                let text = v
                    .get("text")
                    .or_else(|| v.get("output"))
                    .or_else(|| v.get("content"))
                    .or_else(|| v.get("message"))
                    .and_then(|x| x.as_str());
                println!("text: {text:?}");
                println!("(full body: {})", serde_json::to_string_pretty(&v)?);
            }
            Err(e) => println!("chat errored: {e}"),
        }

        // Cleanup the mission dir. We did NOT mutate agents.json
        // (we used the developer's existing agent row), so nothing
        // to undo there.
        let _ = std::fs::remove_dir_all(&mission_dir);
    }

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
