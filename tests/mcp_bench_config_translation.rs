// Integration test for tools/scripts/mcp_bench_setup.sh — verifies the
// commands.json → mcps.json translation produces JSON that
// the daemon's McpClientService::from_path actually accepts.
//
// The script itself does I/O (git clone, install.sh), so the test
// can't run the whole flow. Instead it:
//
//   1. Builds an in-memory minimal `commands.json` matching the
//      shapes mcp-bench actually uses (stdio + http, with cwd +
//      env list).
//   2. Replicates the translation logic the script's python
//      heredoc applies (single source of truth: both this test
//      and the script MUST stay in sync — when changing one,
//      update the other, and run this test).
//   3. Deserialises the result into `McpClientsFile` and validates
//      every spec.
//
// If McpServerSpec evolves and the script's python block doesn't
// follow, this test trips before an operator does at boot time.

use easynet_cli::daemon::execution::mcp::McpClientsFile;
use serde_json::{json, Value};

fn translate_commands_json(commands: &Value, mcp_bench_dir: &str) -> Value {
    // Mirrors the python heredoc in tools/scripts/mcp_bench_setup.sh —
    // any divergence between the two implementations breaks the
    // operator's boot. Keep them in lock-step.
    let servers_dir = format!("{mcp_bench_dir}/mcp_servers");
    let mut servers = Vec::new();
    let obj = commands
        .as_object()
        .expect("commands.json must be an object");
    for (name, entry) in obj {
        let cmd_str = entry.get("cmd").and_then(Value::as_str).unwrap_or("");
        let cwd_rel = entry.get("cwd").and_then(Value::as_str);
        let cwd_abs = cwd_rel.map(|c| {
            // shell-quote-safe enough for test purposes: mirror
            // tools/scripts/mcp_bench_setup.sh's handling of mcp-bench's
            // "../server" cwd values. install.sh places those
            // checkouts under mcp_servers/server.
            if c.starts_with("../") {
                format!("{}/{}", servers_dir, c.trim_start_matches("../"))
            } else {
                format!("{servers_dir}/{c}")
            }
        });

        let (command, args) = if let Some(cwd) = &cwd_abs {
            (
                "sh".to_string(),
                vec!["-c".to_string(), format!("cd {cwd} && exec {cmd_str}")],
            )
        } else {
            let parts: Vec<&str> = cmd_str.split_whitespace().collect();
            (
                parts.first().copied().unwrap_or("").to_string(),
                parts.iter().skip(1).map(|s| s.to_string()).collect(),
            )
        };

        let transport = entry
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or("stdio")
            .to_string();

        let mut spec = json!({
            "name": name,
            "command": command,
            "args": args,
            "env": {},
            "transport": transport,
        });

        if transport == "http" {
            let port = entry.get("port").and_then(Value::as_u64).unwrap_or(3001);
            let endpoint = entry
                .get("endpoint")
                .and_then(Value::as_str)
                .unwrap_or("/mcp");
            spec["url"] = json!(format!("http://127.0.0.1:{port}"));
            spec["endpoint"] = json!(endpoint);
        }

        servers.push(spec);
    }

    json!({"servers": servers})
}

#[test]
fn translated_commands_json_deserialises_into_mcps_file() {
    // A representative slice of mcp-bench's real commands.json —
    // one stdio entry, one http entry — covering the field-shape
    // surface the script must produce.
    let commands = json!({
        "Wikipedia": {
            "cmd": "uv run python -m wikipedia_mcp",
            "env": [],
            "cwd": "../wikipedia-mcp"
        },
        "Google Maps": {
            "cmd": "node dist/cli.js --port 3001",
            "env": ["GOOGLE_MAPS_API_KEY"],
            "cwd": "../mcp-google-map",
            "transport": "http",
            "port": 3001,
            "endpoint": "/mcp"
        }
    });

    let translated = translate_commands_json(&commands, "/tmp/mcp-bench");
    let json_string = serde_json::to_string_pretty(&translated).unwrap();

    // The whole point: the translation MUST produce JSON
    // McpClientService::from_path accepts. If McpServerSpec
    // gains a new required field, this trips and forces the
    // script to be updated in lock-step.
    let parsed: McpClientsFile =
        serde_json::from_str(&json_string).expect("must deserialise into McpClientsFile");
    assert_eq!(parsed.servers.len(), 2);

    // Spot-check each spec — every field the daemon will read
    // must round-trip with the expected value.
    let wiki = parsed
        .servers
        .iter()
        .find(|s| s.name == "Wikipedia")
        .unwrap();
    assert_eq!(wiki.transport, "stdio");
    assert_eq!(wiki.command, "sh");
    assert!(wiki.args[0] == "-c");
    assert!(wiki.args[1].contains("cd ") && wiki.args[1].contains("wikipedia_mcp"));
    assert!(
        wiki.args[1].contains("/tmp/mcp-bench/mcp_servers/wikipedia-mcp"),
        "cwd must point at mcp_servers install dir, got {}",
        wiki.args[1]
    );
    assert!(wiki.url.is_none());
    wiki.validate().expect("Wikipedia spec must validate");

    let gmaps = parsed
        .servers
        .iter()
        .find(|s| s.name == "Google Maps")
        .unwrap();
    assert_eq!(gmaps.transport, "http");
    assert_eq!(gmaps.url.as_deref(), Some("http://127.0.0.1:3001"));
    assert_eq!(gmaps.endpoint, "/mcp");
    gmaps.validate().expect("Google Maps spec must validate");
}

#[test]
fn every_translated_spec_passes_validate() {
    // Defensive: even a malformed entry in commands.json should
    // either produce a spec that validate() accepts or be
    // explicitly flagged. This test guards against silent
    // half-broken configs reaching the daemon.
    let commands = json!({
        "Minimal": {
            "cmd": "python server.py",
            "env": [],
            "cwd": "../minimal"
        }
    });
    let translated = translate_commands_json(&commands, "/tmp/mcp-bench");
    let parsed: McpClientsFile = serde_json::from_value(translated).expect("must deserialise");
    for spec in &parsed.servers {
        spec.validate()
            .unwrap_or_else(|e| panic!("spec {} must validate: {e}", spec.name));
    }
}

#[test]
fn translation_script_file_is_present_and_executable() {
    // Sanity: the script itself exists in-tree. The translation
    // logic above MIRRORS the script's python heredoc; this guard
    // makes sure the script wasn't accidentally deleted while
    // this test continues to enforce the schema.
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("scripts")
        .join("mcp_bench_setup.sh");
    assert!(script.exists(), "tools/scripts/mcp_bench_setup.sh missing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "tools/scripts/mcp_bench_setup.sh not executable"
        );
    }
}
