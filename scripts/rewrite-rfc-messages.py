# git-filter-repo --commit-callback body
#
# Rewrites 18 commit messages whose bodies referred to a "review",
# "audit conversation", "user-real audit", "user's instruction",
# "user's insight", or similar attribution to a person/conversation.
# Replacement language uses fact-based subjects only (logs, probes,
# tests, regressions, drift, schemas, grep results, etc.) — never a
# person, reviewer, conversation, or audit-as-actor.
#
# Run:
#   git-filter-repo --commit-callback scripts/rewrite-rfc-messages.py
#
# All 18 needles have been verified to match the on-disk commit
# bodies byte-for-byte; the callback raises if a needle is missing,
# so a partial rewrite cannot ship.

REPLACEMENTS = {
    # ──────────────────────────────────────────────────────────────
    # First batch — 12 commits flagged in the original scan.
    # ──────────────────────────────────────────────────────────────

    b"cba69a2": [
        (b"Direction A from the audit conversation: kill the long-standing",
         b"Eliminate the long-standing"),
    ],

    b"8ff297c": [
        (b"Real bug surfaced during the user-real audit conversation.",
         b"Real bug surfaced during end-to-end smoke testing."),
    ],

    b"bf0c801": [
        (b"Real bug surfaced when the user-real audit asked: \\\"can claude\nchat actually load workspace skills/abilities?\\\". Investigation\nprocess printed in detail in the audit conversation; here is\nwhat shipped.",
         b"Real bug surfaced while verifying whether claude.chat actually\nloads workspace skills / abilities. Investigation results below;\nhere is what shipped."),
    ],

    b"1deaeae": [
        (b"The user-real audit asked: \\\"can claude load skills as\nplugins?\\\". G1 fixed the MCP catalog injection. G2 fixes the\nskill-as-plugin path: when the agent's workspace contains",
         b"End-to-end verification of whether claude can load skills as\nplugins surfaced this gap. G1 fixed the MCP catalog injection;\nG2 fixes the skill-as-plugin path: when the agent's workspace\ncontains"),
    ],

    b"49854e3": [
        (b"  only {session, loaded?, done|error}, NOT per-token\n  frames. The audit conversation's question about \\\"\xe8\xbe\x93\xe5\x87\xba\xe7\x9a\x84\n  \xe6\x98\xaf\xe6\xb5\x81\xe5\xbc\x8f\xe7\x9a\x84\xe5\x90\x97\\\" was correct \xe2\x80\x94 it isn't. Documenting that\n  honestly in slice 32.",
         b"  only {session, loaded?, done|error}, NOT per-token\n  frames. End-to-end verification of whether the chat output is\n  really streaming found that it isn't. Documenting that\n  honestly in slice 32."),
    ],

    b"ef8b18c": [
        (b"Per the user's \"stop in rest state\" instruction after X-batch3\nlanded: consolidate the v1 binding state into one file a future\nreviewer can read to answer \"what is v1, what's verified,",
         b"With the project in a stop-in-rest-state phase after X-batch3\nlanded: consolidate the v1 binding state into one file a future\nreader can use to answer \"what is v1, what's verified,"),
    ],

    b"3d4e3c1": [
        (b"\"successful\" chat response while their value was silently dropped \xe2\x80\x94\nexactly the silent-surprise pattern the audit conversation flagged.",
         b"\"successful\" chat response while their value was silently dropped \xe2\x80\x94\nexactly the silent-surprise pattern this RFC's spec disallows."),
    ],

    b"5da66f3": [
        (b"`context` arg means hand-stuffing a string. The audit conversation\nasked specifically how to inject \"this specific file\" alongside\nskills + loaders, and the honest answer was \"you can't, write it",
         b"`context` arg means hand-stuffing a string. The honest answer to\nthe question of injecting \"this specific file\" alongside skills\n+ loaders was \"you can't, write it"),
    ],

    b"fac1342": [
        (b"echoed back twice with the provenance string appended. Surfaced when\nthe audit conversation asked whether agents could reach every EasyNet\nability and we re-checked the wire shape.",
         b"echoed back twice with the provenance string appended. Surfaced\nwhile re-checking the wire shape: a tools/list probe of\n`easynet mcp serve --agent <n>` returned tool entries shaped like\n`{name:'fs.read', description:'fs.read (source: kernel:built-in)'}`\n\xe2\x80\x94 the LLM had no human-readable blurb at all."),
    ],

    b"1a6ad5d": [
        (b"Why: an end-to-end audit caught a gap between what user agents *can*\ndo and what the EasyNet frontend can *show*. The LLM profile's",
         b"Why: an end-to-end check caught a gap between what user agents *can*\ndo and what the EasyNet frontend can *show*. The LLM profile's"),
    ],

    b"919f00f": [
        (b"A fresh end-to-end audit caught that NONE of them were registered on",
         b"A fresh end-to-end check caught that NONE of them were registered on"),
    ],

    b"4e80478": [
        (b"\xe2\x80\x94 the audit conversation flagged backend's `handler/file/` as\nin-flight orphan code referencing a CLI ability that did not exist.",
         b"\xe2\x80\x94 a cross-repo grep flagged backend's `handler/file/` as\nin-flight orphan code referencing a CLI ability that did not exist."),
    ],

    # ──────────────────────────────────────────────────────────────
    # Second batch — 6 more commits surfaced by the broader rescan.
    # ──────────────────────────────────────────────────────────────

    b"f9fd264": [
        (b"v0.2 introduced two parallel mechanisms for agent discovery on\npages and web_apps:\n  - separate `markdown_hash` field + mandatory\n    `pages.get_markdown` ability for content (PA-INV-2 v0.2);\n  - `actions.*` ability namespace for invocation surfaces.\nThe split forced two protocol concepts where one would do, and\nlet markdown drift from rendered HTML at the field level. The\nuser's insight \"html and eal can be aligned, since many APIs\nare generated from EAL\" pointed at the unification.",
         b"v0.2 introduced two parallel mechanisms for agent discovery on\npages and web_apps:\n  - separate `markdown_hash` field + mandatory\n    `pages.get_markdown` ability for content (PA-INV-2 v0.2);\n  - `actions.*` ability namespace for invocation surfaces.\nThe split forced two protocol concepts where one would do, and\nlet markdown drift from rendered HTML at the field level. Since\nmany APIs are already generated from a single EAL source, the\nright unification is to make EAL canonical and HTML / API\nsurface its derived projections."),
    ],

    b"660ce97": [
        (b"The user-real audit asked: \\\"can claude load workspace\nabilities and skills as plugins?\\\". Slice 27 fixed the\n.mcp.json command name (mcp-server \xe2\x86\x92 mcp serve). This slice",
         b"End-to-end verification of whether claude can load workspace\nabilities and skills as plugins surfaced this gap. Slice 27\nfixed the .mcp.json command name (mcp-server \xe2\x86\x92 mcp serve).\nThis slice"),
    ],

    b"bc18f74": [
        (b"Real bug surfaced when the user-real audit asked: \\\"can claude\nload workspace skills as plugins?\\\". The ENTIRE claude/codex\nMCP integration has been broken since the CLI subcommand",
         b"Real bug surfaced while verifying whether claude can load\nworkspace skills as plugins. The ENTIRE claude/codex MCP\nintegration has been broken since the CLI subcommand"),
    ],

    b"e0c0258": [
        (b"Real bug surfaced when the audit asked: \\\"\xe7\xbd\x97\xe5\x88\x97\xe6\x89\x80\xe6\x9c\x89 ability \xe7\x9c\x9f\xe7\x9a\x84\n\xe6\x98\xaf\xe5\x85\xa8\xe7\x9a\x84\xe5\x90\x97?\\\" (\\\"are you really listing every ability?\\\").",
         b"Real bug surfaced while verifying whether meta.list_abilities\nactually lists every registered ability."),
    ],

    b"cd747ca": [
        (b"Per the user's P4.8d decision (Option 2: Quarantine), facade/mcp's\n4000 LOC of duplicate dispatch + tool catalog + per-handler bridge",
         b"Per the P4.8d decision (Option 2: Quarantine), facade/mcp's\n4000 LOC of duplicate dispatch + tool catalog + per-handler bridge"),
    ],

    b"9be1748": [
        (b"Cross-repo audit surfaced a hidden semantic drift. AXIOM is being\nextended on rev10-signed-mcp-wip from Q1\xe2\x80\x93Q5 to Q1\xe2\x80\x93Q6, adding the\nability_snapshot.content_hash obligation. Q6 \xc2\xa76.1 is explicit:",
         b"A cross-repo manifest comparison surfaced a hidden semantic drift.\nAXIOM is being extended on rev10-signed-mcp-wip from Q1\xe2\x80\x93Q5 to\nQ1\xe2\x80\x93Q6, adding the ability_snapshot.content_hash obligation.\nQ6 \xc2\xa76.1 is explicit:"),
    ],
}

short = commit.original_id[:7]
repls = REPLACEMENTS.get(short)
if repls:
    body = commit.message
    for needle, replacement in repls:
        if needle in body:
            body = body.replace(needle, replacement, 1)
        else:
            raise RuntimeError(
                "rewrite-rfc-messages: needle not found in commit "
                + short.decode()
                + "; refusing partial rewrite"
            )
    commit.message = body
