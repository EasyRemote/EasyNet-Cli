// EasyNet CLI — real-user smoke reproducer
// =========================================
//
// Run a real-disk / real-binary / real-API exercise of the
// load-bearing abilities (fs.write, fs.read, process.exec,
// shell.run, <agent>.chat) end-to-end through the actual
// Axon LocalRuntime, and print what each one returned. Distinct from
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
// runtime::system_abilities::real_invoke_tests for the deterministic
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
use easynet_axon::invocation::{LocalRuntime, StreamingInvocationHandle};
use serde_json::{json, Value};
use std::sync::Arc;

use easynet_cli::daemon::ability::catalog::{
    build_registry_for_daemon, build_registry_with_runtime, RegistryBuildServices,
    RegistryDaemonBuildConfig,
};
use easynet_cli::runtime::ability_dispatch::AxonAbilityCatalog;
use easynet_cli::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
use easynet_cli::runtime::local_runtime_invoker::{invoke_local_rpc_sync, open_local_stream};
use easynet_cli::runtime::resources::filesystem::{
    resource_ref_for_local_path, FilesystemResourceCapability,
};
use easynet_cli::runtime::system_abilities::agents::chat::ContextLoader;
use easynet_cli::support::async_bridge::{run_blocking, NoRuntimeFallback};

fn target(ability: &str, args: Value) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: ability.to_string(),
        normalized_args: args,
        call_mode: CallMode::Rpc,
        subject: None,
        causal_context: None,
    }
}

struct RuntimeSmoke {
    runtime: Arc<LocalRuntime>,
    catalog: Arc<AxonAbilityCatalog>,
}

impl RuntimeSmoke {
    fn system() -> Self {
        let runtime = LocalRuntime::new();
        let catalog = build_registry_with_runtime(Arc::clone(&runtime));
        Self { runtime, catalog }
    }

    fn daemon(loaders: Option<Arc<Vec<Arc<dyn ContextLoader>>>>) -> Self {
        let runtime = LocalRuntime::new();
        let mut config = RegistryDaemonBuildConfig::new(RegistryBuildServices::fresh());
        config.loaders = loaders;
        config.pages_identity =
            easynet_cli::runtime::system_abilities::resources::pages::PagesIdentity::from_env();
        config.local_runtime = Some(Arc::clone(&runtime));
        let catalog = build_registry_for_daemon(config);
        Self { runtime, catalog }
    }

    fn list_abilities(&self) -> Vec<String> {
        self.catalog.list_abilities()
    }

    fn execute_rpc(&self, target: InvocationTarget) -> anyhow::Result<Value> {
        if target.call_mode != CallMode::Rpc {
            anyhow::bail!("execute_rpc called with non-RPC target");
        }
        invoke_local_rpc_sync(Arc::clone(&self.runtime), target)
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    fn execute_stream(
        &self,
        target: InvocationTarget,
    ) -> anyhow::Result<easynet_axon::invocation::StreamingInvocationHandle> {
        if target.call_mode != CallMode::Stream {
            anyhow::bail!("execute_stream called with non-stream target");
        }
        run_blocking(
            open_local_stream(Arc::clone(&self.runtime), target),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        )
        .map_err(|err| anyhow::anyhow!("{err}"))
    }
}

fn d() -> RuntimeSmoke {
    RuntimeSmoke::system()
}

fn b64(s: &str) -> String {
    let bytes = BASE64_STANDARD.decode(s).unwrap();
    String::from_utf8(bytes).unwrap_or_else(|e| format!("<non-utf8: {e}>"))
}

fn print_stream_frames(
    rt: &tokio::runtime::Runtime,
    label: &str,
    mut stream: StreamingInvocationHandle,
) {
    let stream_start = std::time::Instant::now();
    let mut frame_seq: u64 = 0;
    rt.block_on(async {
        while let Some(frame_result) = stream.next_frame().await {
            match frame_result {
                Ok(frame) => {
                    if !frame.payload.is_empty() {
                        frame_seq += 1;
                        let elapsed = stream_start.elapsed().as_millis();
                        let value: Value = serde_json::from_slice(&frame.payload).unwrap_or_else(
                            |err| json!({"type": "decode_error", "message": err.to_string()}),
                        );
                        let kind = value.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                        let preview = match kind {
                            "done" => {
                                let reply =
                                    value.get("reply").and_then(|r| r.as_str()).unwrap_or("");
                                format!(
                                    "type=done reply={:?} ({}b)",
                                    &reply.chars().take(40).collect::<String>(),
                                    reply.len()
                                )
                            }
                            other => format!("type={other}"),
                        };
                        println!("  [t+{elapsed:>5}ms #{frame_seq:>2}] {preview}");
                    }
                    if frame.terminal {
                        break;
                    }
                }
                Err(err) => {
                    println!("  stream error: {err}");
                    break;
                }
            }
        }
    });
    println!(
        "{label} stream finished after {} frames in {}ms",
        frame_seq,
        stream_start.elapsed().as_millis()
    );
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
            "resource_ref": resource_ref_for_local_path(
                &out,
                FilesystemResourceCapability::Write,
            )?,
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
        json!({
            "resource_ref": resource_ref_for_local_path(
                &out,
                FilesystemResourceCapability::Read,
            )?,
            "encoding": "utf8"
        }),
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
            d().execute_rpc(target("shell.run", json!({"command": "rm /tmp/x"})))
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
        //    cli::mission_runs::root_dir() returns:
        //    `$HOME/.easynet/missions/runs/`.
        let mission_id = format!("smoke-{}", std::process::id());
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let mission_root = home.join(".easynet").join("missions").join("runs");
        let mission_dir = mission_root.join(&mission_id);
        std::fs::create_dir_all(&mission_dir)?;

        // 2. Build registry from the developer's ACTUAL ~/.easynet/agents.json
        //    so existing `claude` / `codex` rows are picked up. This bin is
        //    deliberately running against the developer's real config — the
        //    EASYNET_REAL_CHAT_OK guard signals consent.
        let smoke = RuntimeSmoke::daemon(Some(Arc::new(Vec::new())));
        let advertised = smoke.list_abilities();
        let chat_ability = advertised.iter().find(|n| n.ends_with(".chat"));
        let chat_ability = match chat_ability {
            Some(n) => n.clone(),
            None => {
                println!("(skipped: no <agent>.chat registered. Add one with");
                println!(" `easynet agent add <name> --type claude-code` first.)");
                return Ok(());
            }
        };
        println!("Will invoke: {chat_ability}");

        // Workspace-tool-injection probe: ask claude to list every
        // MCP tool whose name contains "audit". Post-G1, the
        // agent's own claude.audit-test-ability MUST be visible
        // through the EasyNet MCP server. Pre-G1 it would only
        // see device-profile tools (fs.read etc).
        let prompt = "Use the /audit-canary-plugin skill if available. \
                      Reply with the exact text the skill instructs you to. \
                      If the skill doesn't exist, reply with: SKILL_NOT_AVAILABLE.";
        println!("Prompt: {prompt:?}");
        println!("Calling {chat_ability} (this will spawn the real CLI)...");

        let chat_for_invoke = chat_ability.clone();
        let chat_mission_id = mission_id.clone();
        let chat_mission_dir = mission_dir.clone();
        let result = rt.block_on(async move {
            tokio::task::spawn_blocking(move || {
                let _mission_ctx = easynet_cli::runtime::enter_mission_context_for_current_thread(
                    chat_mission_id,
                    chat_mission_dir,
                );
                smoke.execute_rpc(target(
                    &chat_for_invoke,
                    json!({"prompt": prompt, "stream": false}),
                ))
            })
            .await
            .unwrap()
        });

        match result {
            Ok(v) => {
                println!(
                    "response keys: {:?}",
                    v.as_object().map(|o| o.keys().collect::<Vec<_>>())
                );
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

        // ── context-injection probe ───────────────────────────
        // Does the `context` field actually reach the LLM?
        // Pass a unique string in `context`, ask the LLM to
        // echo it back. If compose_prompt drops it the LLM
        // can't possibly reply with the canary.
        println!("\n=== chat `context` field reality check ===");
        let canary = format!("CANARY-{}-CTX", std::process::id());
        let chat_for_ctx = chat_ability.clone();
        // Match the daemon's default loader chain so this test
        // mirrors what an operator hitting `easynet agent send`
        // would see — user_profile + schedule + memory loaders
        // attached, instead of an empty Vec like the other
        // smoke probes use.
        let schedule_svc =
            Arc::new(easynet_cli::runtime::execution::schedule::ScheduleService::new());
        let default_loaders = Arc::new(
            easynet_cli::runtime::system_abilities::resources::context::loaders::default_loaders(
                Arc::clone(&schedule_svc),
            ),
        );
        let ctx_runtime = RuntimeSmoke::daemon(Some(default_loaders));
        let canary_for_thread = canary.clone();
        let ctx_mission_id = mission_id.clone();
        let ctx_mission_dir = mission_dir.clone();
        let ctx_result = rt.block_on(async {
            tokio::task::spawn_blocking(move || {
                let _mission_ctx = easynet_cli::runtime::enter_mission_context_for_current_thread(
                    ctx_mission_id,
                    ctx_mission_dir,
                );
                ctx_runtime.execute_rpc(InvocationTarget {
                    scope: TargetScope::Local,
                    ability: chat_for_ctx,
                    normalized_args: json!({
                        "prompt": "What single all-caps token from the previous discussion contains the substring `CANARY-`? Reply with that token only, no other text.",
                        "context": format!("In this session the agreed-upon canary token is `{canary_for_thread}`. Remember it for any future verification."),
                        "stream": false,
                    }),
                    call_mode: CallMode::Rpc,
                    subject: None,
                    causal_context: None,
                })
            })
            .await
            .unwrap()
        });
        match ctx_result {
            Ok(v) => {
                let reply = v.get("reply").and_then(|r| r.as_str()).unwrap_or("(none)");
                println!("Canary expected: {canary}");
                println!("Canary in reply : {reply}");
                if reply.contains(&canary) {
                    println!("✓ context injection works: LLM read the `context` field");
                } else {
                    println!("✗ context injection FAILED: reply did not contain the canary");
                }
                println!(
                    "context_used: {}",
                    v.get("context_used")
                        .map(|c| c.to_string())
                        .unwrap_or_default()
                );
            }
            Err(e) => println!("context probe failed: {e}"),
        }

        // ── G1-handler reality check ──────────────────────────
        println!("\n=== claude.audit-test-ability direct invocation ===");
        // Call the agent's own declared ability through LocalRuntime.
        // Pre-fix this returned NOT_FOUND. Post-fix the chat
        // fallback handler dispatches to claude.chat with a
        // synthesised prompt; the reply comes back as
        // {result: <agent reply>, fulfilled_by: "agent_chat", ...}.
        let ability_runtime = RuntimeSmoke::daemon(Some(Arc::new(Vec::new())));
        let direct_result = rt.block_on(async {
            tokio::task::spawn_blocking(move || {
                ability_runtime.execute_rpc(InvocationTarget {
                    scope: TargetScope::Local,
                    ability: "claude.audit-test-ability".to_string(),
                    normalized_args: json!({}),
                    call_mode: CallMode::Rpc,
                    subject: None,
                    causal_context: None,
                })
            })
            .await
            .unwrap()
        });
        match direct_result {
            Ok(v) => {
                println!("Direct ability call SUCCESS:");
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
            Err(e) => {
                println!("Direct ability call FAILED: {e}");
            }
        }

        // ── codex.chat streaming check ─────────────────────────
        // If codex.chat is registered, run the same streaming
        // probe against it. Pre-fix slice 33, codex's driver
        // didn't fan out per-line progress to the broadcast
        // channel — only claude_code.rs did. This block proves
        // codex now streams too (separate code path through
        // CodexExecAdapter::invoke).
        if advertised.iter().any(|n| n == "codex.chat") {
            println!("\n=== codex.chat STREAM mode (frame-by-frame) ===");
            let dispatcher_for_codex = RuntimeSmoke::daemon(Some(Arc::new(Vec::new())));
            let codex_target = InvocationTarget {
                scope: TargetScope::Local,
                ability: "codex.chat".to_string(),
                normalized_args: json!({
                    "prompt": "Count from 1 to 5, one number per line.",
                    "stream": true,
                }),
                call_mode: CallMode::Stream,
                subject: None,
                causal_context: None,
            };
            match dispatcher_for_codex.execute_stream(codex_target) {
                Ok(stream) => print_stream_frames(&rt, "Codex", stream),
                Err(e) => {
                    println!("codex.chat stream failed: {e}");
                }
            }
        } else {
            println!("\n=== codex.chat STREAM mode: SKIPPED (no codex agent registered) ===");
        }

        // ── streaming reality check (claude) ──────────────────
        println!("\n=== claude.chat STREAM mode (frame-by-frame) ===");
        let dispatcher_for_stream = RuntimeSmoke::daemon(Some(Arc::new(Vec::new())));
        let stream_target = InvocationTarget {
            scope: TargetScope::Local,
            ability: chat_ability.clone(),
            normalized_args: json!({
                "prompt": "Count from 1 to 5, one number per line.",
                "stream": true,
            }),
            call_mode: CallMode::Stream,
            subject: None,
            causal_context: None,
        };
        {
            let _mission_ctx = easynet_cli::runtime::enter_mission_context_for_current_thread(
                mission_id.clone(),
                mission_dir.clone(),
            );
            let stream = dispatcher_for_stream
                .execute_stream(stream_target)
                .expect("execute_stream");
            print_stream_frames(&rt, "Claude", stream);
        }

        // Cleanup the mission dir. We did NOT mutate agents.json
        // (we used the developer's existing agent row), so nothing
        // to undo there.
        let _ = std::fs::remove_dir_all(&mission_dir);
    }

    println!("\n=== meta.list_abilities completeness audit ===");
    let meta_runtime = RuntimeSmoke::daemon(Some(Arc::new(Vec::new())));
    let registered_names: std::collections::BTreeSet<String> =
        meta_runtime.list_abilities().into_iter().collect();
    println!(
        "LIVE registry has {} abilities (registered_rpc/stream/bidi):",
        registered_names.len()
    );

    let meta_resp = meta_runtime
        .execute_rpc(target("meta.list_abilities", json!({})))
        .expect("meta.list_abilities");
    let meta_names: std::collections::BTreeSet<String> = meta_resp["abilities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    println!(
        "meta.list_abilities returned {} abilities:",
        meta_names.len()
    );

    println!("\nIn LIVE registry but NOT in meta.list_abilities:");
    let only_registered: Vec<&String> = registered_names.difference(&meta_names).collect();
    for n in &only_registered {
        println!("  - {n}");
    }
    println!("\nIn meta.list_abilities but NOT in LIVE registry:");
    let only_meta: Vec<&String> = meta_names.difference(&registered_names).collect();
    for n in &only_meta {
        println!("  + {n}");
    }
    if only_registered.is_empty() && only_meta.is_empty() {
        println!("\n✓ meta.list_abilities matches the live registry exactly.");
    } else {
        println!(
            "\n⚠ meta.list_abilities and the live registry disagree on {} entries.",
            only_registered.len() + only_meta.len()
        );
    }

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
