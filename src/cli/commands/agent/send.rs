// EasyNet CLI — `easynet agent` conversation surface: send, session picker, usage/markdown rendering
// Split from cli/agent.rs (F-033 / T4.6); bodies are move-only.

use console::style;

use super::*;

fn resolve_session_id(args: &SendArgs) -> anyhow::Result<Option<String>> {
    use crate::daemon::persistence::chat_sessions;

    // clap's ArgGroup already enforces "at most one of these" but
    // we double-check defensively in case future refactors break
    // the group decl. Cheaper than discovering the silent failure
    // mode in production.
    let n_flags =
        (args.follow as u8) + args.session_id.as_ref().map_or(0, |_| 1) + (args.resume as u8);
    if n_flags > 1 {
        anyhow::bail!(
            "--follow, --session-id, and --resume are mutually exclusive; pass at most one"
        );
    }

    if let Some(explicit) = args.session_id.as_deref() {
        let trimmed = explicit.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--session-id is empty (shell expansion accident?)");
        }
        return Ok(Some(trimmed.to_string()));
    }

    if args.follow {
        match chat_sessions::latest_session(&args.name)? {
            Some(sid) => return Ok(Some(sid)),
            None => anyhow::bail!(
                "agent '{}' has no recorded sessions yet — \
                 send a fresh prompt without --follow first",
                args.name
            ),
        }
    }

    if args.resume {
        let sessions = chat_sessions::list_sessions(&args.name)?;
        if sessions.is_empty() {
            anyhow::bail!(
                "agent '{}' has no recorded sessions yet — \
                 send a fresh prompt without --resume first",
                args.name
            );
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            anyhow::bail!(
                "--resume is interactive; stdin is not a terminal. \
                 Use --session-id <UUID> instead — list candidates with \
                 'easynet agent chat-history {} list'",
                args.name
            );
        }
        return prompt_session_picker(&args.name, &sessions).map(Some);
    }

    Ok(None)
}

/// Arrow-key TUI picker for `--resume`. Backed by `dialoguer`'s
/// `Select` (a thin wrapper over `console`, already a direct dep),
/// so the picker reuses the same terminal backend as the rest of
/// the CLI — no second TUI stack pulled in.
///
/// UX:
///   * ↑/↓ to move, Enter to confirm, Esc / q / Ctrl-C to abort.
///   * Cursor starts on the most-recent session (index 0; the
///     same id `--follow` would resume), so the common case
///     ("just continue the latest") is one Enter away.
///   * Each row renders `<short-id>  N turns  <since>  <preview>`.
///     Short id = first 8 chars of the UUID, enough to disambiguate
///     human-scale session counts without making the row 80 cols
///     wide.
///   * Cap at 50 most-recent sessions on screen — the picker grows
///     unwieldy past that and operators with hundreds of sessions
///     should pin via `--session-id` (which they already have to
///     copy from `agent chat-history list`).
///
/// Stdin is already verified to be a TTY by the caller. Aborts
/// (Esc / q / Ctrl-C / no choice) surface as a typed Err so
/// `agent send` doesn't silently hand back to the user with no
/// message.
fn prompt_session_picker(
    agent: &str,
    sessions: &[crate::daemon::persistence::chat_sessions::SessionDescriptor],
) -> anyhow::Result<String> {
    use dialoguer::theme::ColorfulTheme;
    use dialoguer::Select;

    const PICKER_CAP: usize = 50;
    let visible = &sessions[..sessions.len().min(PICKER_CAP)];

    let labels: Vec<String> = visible
        .iter()
        .map(|s| {
            let short_id: String = s.session_id.chars().take(8).collect();
            let preview = if s.prompt_preview.is_empty() {
                String::from("(no prompt yet)")
            } else {
                s.prompt_preview.clone()
            };
            format!(
                "{}  {} turns  {}  {}",
                short_id,
                s.turn_count,
                relative_age(&s.last_turn_at),
                preview,
            )
        })
        .collect();

    let header = if sessions.len() > PICKER_CAP {
        format!(
            "Pick a prior session for {agent} (showing latest {PICKER_CAP} of {}; \
             pin older ones via --session-id <UUID>)",
            sessions.len()
        )
    } else {
        format!(
            "Pick a prior session for {agent} ({} session{})",
            sessions.len(),
            if sessions.len() == 1 { "" } else { "s" }
        )
    };

    let chosen = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(header)
        .items(&labels)
        .default(0)
        .interact_opt()
        .map_err(|e| anyhow::anyhow!("session picker io error: {e}"))?;

    match chosen {
        Some(i) => Ok(visible[i].session_id.clone()),
        None => anyhow::bail!("session picker aborted by user"),
    }
}

/// Format an RFC3339 timestamp as a short relative-age string
/// ("5m ago", "3h ago", "2d ago"). Used by the resume picker so
/// each row stays scannable. Falls back to the raw timestamp if
/// it can't be parsed — bad clock data is a banner-class problem
/// but we don't want it to break `--resume`.
fn relative_age(ts: &str) -> String {
    let parsed = match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt,
        Err(_) => return ts.to_string(),
    };
    let elapsed = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
    let secs = elapsed.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

pub(super) fn run_send(args: SendArgs) -> anyhow::Result<()> {
    // Validate through the daemon's Axon ability surface so the CLI
    // does not own a parallel registry read path.
    let gateway = agent_state_read_gateway();
    let _row = daemon_agent_row(gateway.as_ref(), &args.name)?;

    // `--resume` is picker-only — single job, no prompt allowed.
    // Validate this BEFORE resolving the session id so we don't
    // open the TTY picker just to throw the result away.
    if args.resume && args.prompt.is_some() {
        anyhow::bail!(
            "`--resume` does not take a PROMPT; it only sets the latest \
             session. Run `easynet agent send {} --resume` to pick a \
             session, then `agent send {0} --follow \"<msg>\"` to send.",
            args.name
        );
    }

    // Resolve the session_id BEFORE we kick off mission machinery.
    // The chat ability mints a fresh id when none is supplied; a
    // concrete id triggers the resume path on the daemon side.
    let resolved_session_id = resolve_session_id(&args)?;

    // Two prompt regimes after the early `--resume + prompt`
    // rejection above:
    //
    //   `--resume` (no prompt) → picker → set latest pointer → exit.
    //   anything else          → prompt required (send a new turn).
    let prompt = match args.prompt.as_deref() {
        Some(p) => p.to_string(),
        None => {
            if !args.resume {
                anyhow::bail!(
                    "PROMPT is required (omit only when `--resume` is the \
                     sole session flag, in which case the picker just sets \
                     the latest pointer)."
                );
            }
            let sid = resolved_session_id
                .clone()
                .expect("resume path always returns a session id");
            crate::daemon::persistence::chat_sessions::set_latest_session(&args.name, &sid)?;
            eprintln!(
                "  {} {} {}",
                style("[agent-send]").dim(),
                style("set latest session →").dim(),
                style(&sid).cyan(),
            );
            eprintln!(
                "  {}",
                style(format!(
                    "next `easynet agent send {} --follow \"...\"` will land on this session.",
                    args.name
                ))
                .dim(),
            );
            return Ok(());
        }
    };

    // User-visible counterpart to the doc-comment ontology reference.
    // Tells the user exactly what path their command is taking, so they
    // can reason about why a mission run dir appears, why MCP audit
    // lines may show up, and why the dispatch invariant assertion may
    // fire if anything is misconfigured.
    eprintln!(
        "  {} {}",
        style("[agent-send]").dim(),
        style("dispatching via mission runtime").dim(),
    );
    if let Some(sid) = resolved_session_id.as_deref() {
        eprintln!(
            "  {} {} {}",
            style("[agent-send]").dim(),
            style("resume session").dim(),
            style(sid).cyan(),
        );
    }

    // Compose the prompt: fold optional `--context` into the prompt body
    // BEFORE constructing the EAL source, so the prompt that ends up in
    // the EAL string literal is exactly the prompt the agent will see.
    let composed_prompt = match args.context.as_deref() {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!("{prompt}\n\n## Context (previous discussion)\n\n{ctx}\n")
        }
        _ => prompt.clone(),
    };

    // Build the single-line EAL mission source. The mission name is
    // `agent-send`; the binding is `__reply` so the result can be
    // pulled out of `MissionRunResult.bound_vars`. `eal_string_literal`
    // can fail if the user's prompt contains an embedded NUL byte — we
    // surface that as a CLI error rather than silently truncating.
    let eal_source =
        build_agent_send_eal_source(&args.name, &composed_prompt, resolved_session_id.as_deref())?;

    let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()?;
    let value =
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_subject_timeout(
        crate::daemon::ability::builtins::automation::mission::ABILITY_RUN,
        serde_json::json!({
            "source": eal_source,
            "label": format!("agent send {}", args.name),
        }),
        &local_device_ura,
        std::time::Duration::from_secs(3600),
    )?;
    let result: crate::daemon::ability::builtins::automation::mission::MissionRunResponse =
        serde_json::from_value(value).map_err(|error| {
            anyhow::anyhow!("mission.run returned an invalid response: {error}")
        })?;

    // Pull the agent's reply out of the mission's bound vars. The
    // mission ability returns a JSON object. Two shapes can appear:
    //   * `<agent>.chat` (the invoke_direct_with_progress path):
    //     `{session_id, reply, tool_calls, usage, skills_loaded, ...}`
    //   * non-chat verbs (the send_to_agent shell-out path):
    //     `{ok, agent, output, model, duration_ms}`
    // The user-visible reply lives in `reply` for chat and `output`
    // for shell-out — try both.
    let reply_obj = match result.outputs.get("__reply") {
        Some(serde_json::Value::Object(obj)) => Some(obj.clone()),
        _ => None,
    };
    let reply_text: String = match &reply_obj {
        Some(obj) => obj
            .get("reply")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("output").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::Value::Object(obj.clone()).to_string()),
        None => match result.outputs.get("__reply") {
            Some(other) => other.to_string(),
            None => String::new(),
        },
    };
    // Server-minted session id (when caller passed none) or echoed-back
    // (when the caller pinned one via --follow / --session-id). Echoed
    // to the user so they can copy it for a later --session-id call.
    let response_session_id: Option<String> = reply_obj
        .as_ref()
        .and_then(|obj| obj.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    eprintln!();
    eprintln!(
        "  {} {} {}",
        style(&args.name).white().bold(),
        style("done in").dim(),
        style(format!("{:.1}s", result.meta.duration_ms as f64 / 1000.0)).cyan(),
    );

    // Token line: read the mission runtime's own aggregate. The CLI
    // presentation layer must not inspect nested agent run directories or
    // guess which sibling run belongs to this mission.
    if let Some(usage) = result.meta.token_usage.as_ref() {
        let total_in = usage.input_tokens + usage.cache_read_tokens + usage.cache_creation_tokens;
        eprintln!(
            "  {} in={} out={} cache_read={} cache_write={} turns={} cost=${:.4}",
            style("tokens").dim(),
            style(total_in).cyan(),
            style(usage.output_tokens).cyan(),
            style(usage.cache_read_tokens).dim(),
            style(usage.cache_creation_tokens).dim(),
            style(usage.num_turns).dim(),
            usage.total_cost_usd,
        );
    }

    // "saved" path is the **mission** run dir, not the nested agent run
    // dir. The mission run dir is the artefact users should reference —
    // it contains source.eal, ir.json, trace.json, meta.json, and is
    // where the (currently None) `ability_graph_traces` field will land
    // when v2 ships.
    eprintln!(
        "  {} {}",
        style("saved").dim(),
        style(&result.run_dir).cyan(),
    );
    if let Some(sid) = response_session_id.as_deref() {
        eprintln!(
            "  {} {}  {}",
            style("session").dim(),
            style(sid).cyan(),
            style("(use --follow to continue, --session-id <UUID> to pin)").dim(),
        );
    }
    eprintln!();

    // Turn persistence happens inside the chat ability handler
    // (`chat_ability::invoke_direct_with_progress` →
    // `write_turn_best_effort`) so hub-routed and CLI-routed chats
    // share one transcript writer. Writing here too would record the
    // same turn twice — this path reaches the handler via the
    // `{agent}.chat(...)` mission above.

    // Render the agent's final reply as markdown when stdout is a TTY;
    // otherwise print raw text so piping into other tools stays clean.
    if console::Term::stdout().is_term() {
        let skin = build_markdown_skin();
        let compact = compact_markdown(&reply_text);
        skin.print_text(&compact);
    } else {
        println!("{}", reply_text);
    }
    Ok(())
}

/// Quote a string as a valid EAL string literal: wrap in double quotes
/// and escape every character the EAL lexer would otherwise consume or
/// that downstream consumers (the deployed ability runtime, agent
/// prompts) might mis-handle.
///
/// EAL's lexer (`src/eal/lexer.rs::read_string`) treats `\\<char>` as
/// "skip one byte after the backslash" rather than performing real
/// escape decoding (locked contract — see iter-4 audit notes), so we
/// only need to defang the characters that would terminate the literal
/// (`"`) or change how the lexer counts bytes (`\\`). The remaining
/// escapes (`\n`, `\r`, `\t`) keep the generated EAL source readable
/// when the user pastes a multi-line prompt.
///
/// We additionally reject ASCII NUL: while EAL's lexer would store it
/// happily, downstream consumers — agent CLIs that treat the prompt as
/// a C string, ability runtimes that exec via shell — silently
/// truncate at the first `\0`. Better to fail loud at the call site
/// (`run_send`) than to deliver a half-prompt.
pub(super) fn eal_string_literal(s: &str) -> anyhow::Result<String> {
    if s.contains('\0') {
        anyhow::bail!(
            "prompt contains an embedded NUL byte (U+0000); strip it before sending — \
             downstream agent CLIs treat NUL as end-of-string and would silently truncate"
        );
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

fn build_agent_send_eal_source(
    agent_name: &str,
    composed_prompt: &str,
    session_id: Option<&str>,
) -> anyhow::Result<String> {
    let agent = eal_string_literal(agent_name)?;
    let prompt = eal_string_literal(composed_prompt)?;
    match session_id {
        Some(sid) => Ok(format!(
            "mission \"agent-send\" {{\n    let __reply = {agent}.chat(prompt: {prompt}, session_id: {sid})\n}}\n",
            sid = eal_string_literal(sid)?,
        )),
        None => Ok(format!(
            "mission \"agent-send\" {{\n    let __reply = {agent}.chat(prompt: {prompt})\n}}\n",
        )),
    }
}

/// Build a custom termimad skin: compact spacing, colourful header levels,
/// high-contrast bold, bright code highlighting.
fn build_markdown_skin() -> termimad::MadSkin {
    use termimad::crossterm::style::{Attribute, Color};
    use termimad::{CompoundStyle, LineStyle, MadSkin, StyledChar};

    let mut skin = MadSkin::default();

    // Headers — each level gets a distinct bright colour. No underline, no
    // centring, no background — just bold colour so the hierarchy pops
    // without wasting vertical space.
    let header_colours = [
        Color::Cyan,     // H1
        Color::Magenta,  // H2
        Color::Yellow,   // H3
        Color::Blue,     // H4
        Color::Green,    // H5
        Color::DarkCyan, // H6
        Color::DarkMagenta,
        Color::DarkYellow,
    ];
    for (i, h) in skin.headers.iter_mut().enumerate() {
        *h = LineStyle::default();
        h.compound_style = CompoundStyle::with_fg(*header_colours.get(i).unwrap_or(&Color::White));
        h.compound_style.add_attr(Attribute::Bold);
    }

    // Bold / italic / inline code: bright colours for contrast.
    skin.bold = CompoundStyle::with_fg(Color::White);
    skin.bold.add_attr(Attribute::Bold);

    skin.italic = CompoundStyle::with_fg(Color::Cyan);
    skin.italic.add_attr(Attribute::Italic);

    skin.inline_code = CompoundStyle::with_fgbg(Color::Yellow, Color::Reset);
    skin.inline_code.add_attr(Attribute::Bold);

    // Code block: subtle grey background + bright foreground.
    skin.code_block.compound_style = CompoundStyle::with_fgbg(
        Color::Rgb {
            r: 220,
            g: 220,
            b: 220,
        },
        Color::Rgb {
            r: 30,
            g: 30,
            b: 40,
        },
    );

    // Bullets and other decorations.
    skin.bullet = StyledChar::from_fg_char(Color::Cyan, '▸');
    skin.quote_mark = StyledChar::from_fg_char(Color::Blue, '▌');
    skin.horizontal_rule = StyledChar::from_fg_char(Color::DarkGrey, '─');

    // Table borders — termimad's default `STANDARD_TABLE_BORDER_CHARS`
    // already uses the same Unicode box-drawing characters as
    // `comfy_table::presets::UTF8_FULL_CONDENSED` (│ ─ ┌┐└┘ ┬┴├┤┼), which
    // is the style used by every other `easynet` table (`output::table`).
    // We can't match the `╞═╪╡` header separator or `┆` dashed inside
    // rulers because `TableBorderChars` is uniform, but the overall look
    // is consistent. We only need to colour the border to match the dim
    // grey the EasyNet CLI uses for table chrome.
    skin.table.compound_style.set_fg(Color::DarkGrey);

    skin
}

/// Collapse consecutive blank lines down to a single blank line and strip
/// leading/trailing blanks, so the rendered output stays compact without
/// fighting termimad's layout engine.
fn compact_markdown(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut prev_blank = true; // treat start-of-doc as already-blank
    for line in src.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue; // skip duplicate blank lines
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = is_blank;
    }
    // Trim trailing blank line.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::eal::parser::ast::{FieldValue, Statement, TargetKind};

    use super::*;

    #[test]
    fn agent_send_eal_quotes_non_identifier_agent_receiver() {
        let source = build_agent_send_eal_source("er_agent_20260719-071307", "hello", None)
            .expect("agent send source");

        assert!(source.contains(r#""er_agent_20260719-071307".chat("#));
        assert!(!source.contains("er_agent_20260719-071307.chat("));

        let parsed = crate::eal::parser::parse(&source).expect("generated source must parse");
        let Statement::LetCall { call, .. } = &parsed.mission.statements[0] else {
            panic!("agent send should generate one let-bound member call");
        };
        assert_eq!(call.target_kind, TargetKind::Agent);
        assert_eq!(
            call.target_node.as_deref(),
            Some("er_agent_20260719-071307")
        );
        assert_eq!(call.function_name, "chat");
    }

    #[test]
    fn agent_send_eal_preserves_session_id_as_named_arg() {
        let source = build_agent_send_eal_source(
            "claude",
            "hello",
            Some("8f7dd3b3-992a-48c7-9ecf-e1c9821eda4b"),
        )
        .expect("agent send source");

        let parsed = crate::eal::parser::parse(&source).expect("generated source must parse");
        let Statement::LetCall { call, .. } = &parsed.mission.statements[0] else {
            panic!("agent send should generate one let-bound member call");
        };
        let session = call
            .arguments
            .iter()
            .find(|field| field.key == "session_id")
            .expect("session_id argument");
        match &session.value {
            FieldValue::String(value) => {
                assert_eq!(value, "8f7dd3b3-992a-48c7-9ecf-e1c9821eda4b")
            }
            value => panic!("session_id must be a string literal, got {value:?}"),
        }
    }
}
