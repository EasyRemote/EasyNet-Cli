---
name: easynet-collaborate
description: Discover and invoke abilities owned by other agents on this device or the EasyNet federation. Use when the user asks for live data you can't produce, names a domain you have no skill for, or explicitly addresses another agent — OR when YOU notice you're about to fabricate live data, repeat work another agent already specializes in, or paper over a tool gap with a guess.
allowed-tools: [mcp__easynet]
---

# EasyNet Collaborate

You are part of an EasyNet device with other agents and (when joined) a wider federation. Three discovery tiers are reachable through your `<agent>.discover` and `<agent>.invoke` ability — try them in this order before you reason your way around a tool gap.

## When This Skill Activates

### User-prompted triggers

- The user asks for live data your model can't produce on its own (weather, exchange rates, news, latest code/config state on this machine).
- The user names a domain you have no skill for ("transcribe audio", "render a CAD drawing", "compile this Rust").
- The user explicitly addresses another agent ("ask codex", "have claude do it", "the other agent").
- The user names something you'd normally do but a peer obviously specializes in.

### Self-prompted triggers — activate this skill yourself when YOU notice

- You are about to fabricate a fact instead of looking it up (rates, versions, file states, the user's own data).
- You are repeating a kind of query you just made — "I've done this twice this turn" is a signal an ability already exists.
- You are about to do work outside your specialty (audio, video, ML inference, niche compilation) — a peer agent on the device may already wrap it.
- You hit "I can't do that" and you have not yet checked the catalog. Don't surface that line until tier 1+2 came back empty.
- The user's task crosses a `legacy self alias` boundary you can feel (e.g., they asked you to "have claude review", you ARE claude — you should not delegate to yourself, but you SHOULD discover what abilities your own bundle exposes that you might be skipping).

## Process

### 1. Discover the right tier

Call `<agent>.discover` with the smallest scope that could carry the answer. Walk outward only when the previous tier came back empty.

```
mcp__easynet → <agent>.discover { "scope": "self" }
                                 ↑ your own published abilities + chat
mcp__easynet → <agent>.discover { "scope": "device" }
                                 ↑ all agents on this machine
mcp__easynet → <agent>.discover { "scope": "easynet" }
                                 ↑ federation (only if device joined; may
                                   return federation_not_joined or
                                   federation_unavailable typed errors)
```

Filter rules you apply locally:

- Keep entries whose name matches `<other-agent>.<verb>` for cross-agent calls.
- Drop your own `legacy self alias.chat` and your other `legacy self alias.<verb>` — those are how callers reach you, not how you reach others.
- Keep `host.*` / `fs.*` / `shell.*` / `http.*` / `device.process.exec` if you need a host primitive.
- Skip daemon-internal namespaces unless you specifically need them: `runtime.*`, `federation.*`, `device.fleet.pty_session_*`, `device.a2a.bridge.*`, `device.mcp.bridge.*`.

### 2. Invoke

Two surfaces. Pick the simpler one for one-off calls; use EAL for multi-step composition.

**Single ability call** — use `<agent>.invoke`:

```
mcp__easynet → <agent>.invoke {
  "ability": "claude.weather",
  "args":    { "location": "Beijing" }
}
```

**Multi-step composition** — use `device.mission.run` (EAL program):

```
mcp__easynet → device.mission.run {
  "source": "mission \"weather-and-quip\" {\n  let w = claude.weather(location: \"Beijing\")\n  let q = codex.quip(topic: \"weather\")\n  print(w)\n  print(q)\n}",
  "label":  "weather-quip"
}
```

EAL infers dependencies from variable references — independent steps run in parallel.

### 3. Output

Surface the ability's result, not raw JSON. If the call fails, tell the user briefly and what tier failed (`federation_not_joined` is a different fix from "ability returned an error").

## Tool reference

| Canonical | Note |
|---|---|
| `<agent>.discover` | per-agent introspection alias for `device.meta.list_abilities` |
| `<agent>.invoke` | per-agent dispatch alias for `device.mcp.bridge.call_tool` |
| `device.mission.run` | run an EAL program (compose multiple agent calls) |
| `device.mission.track <run_id>` | poll the persisted state of a long-running mission |
| `device.mission.cancel <run_id>` | flip an in-flight mission to cancelled |

## Examples

### Example: cross-agent delegation (user-prompted)

**Input:**
```
User: what's the weather in Beijing?
```

**Process:**
```
You: <agent>.discover { "scope": "device" }
    → returns claude.weather among others
You: <agent>.invoke { "ability": "claude.weather", "args": { "location": "Beijing" } }
    → "Beijing: 18°C, clear skies"
```

**Output:**
```
"Beijing is 18°C with clear skies right now."
```

### Example: self-prompted catch (no explicit user instruction)

**Input:**
```
User: write a one-paragraph summary of the latest macOS release notes
You start drafting from training data, then catch yourself: "I don't have
the actual release notes content; I'd be inventing version numbers and
features."
```

**Process:**
```
You: <agent>.discover { "scope": "device" }
    → no transcribe/fetch ability for Apple release notes
You: <agent>.discover { "scope": "easynet" }
    → federation_not_joined  (typed error)
You: tell user honestly — "I don't have a way to fetch the live release
     notes; I can summarize what I know up to my training cutoff if that's
     useful, or you can paste the notes."
```

The self-prompted trigger fired even though the user didn't say "look it up" — the activation came from noticing the about-to-fabricate.

### Example: federation tier failing

**Process:**
```
You: <agent>.discover { "scope": "easynet" }
    → { "error": "federation_unavailable", "reason": "hub call failed: timeout" }
You: fall back to device tier; if also empty, surface to user.
```

Don't retry tier 3 in a loop — one attempt, fall back, move on.

## Fallback: direct Axon SDK (only when EasyNet daemon is unavailable)

If `<agent>.discover` returns daemon errors and the user has explicitly said they're not running through `easynet-daemon`, the Axon Python SDK can talk to a local `axon-runtime` directly. The bundled script `${CLAUDE_SKILL_DIR}/scripts/axon-invoke.sh` wraps this path.

Do NOT use this path when EasyNet daemon is up. The whole point of EasyNet is the daemon's discovery + dispatch + audit trail. The SDK fallback exists for axon-only deployments and for debugging the daemon itself.

## What NOT to do

- ❌ Hardcode an ability name. Always go through `<agent>.discover` first; agents come and go.
- ❌ Try to invoke `<other-agent>.chat` to fulfill a specific need — that's a wasteful recursive chat. Call the specific verb instead.
- ❌ Loop discover→invoke more than ~3 times in one turn. If it isn't there, surface that.
- ❌ Promise the user a result without actually invoking. Always call before replying.
- ❌ Ignore typed errors from discover (`federation_not_joined`, `federation_unavailable`). They name the fix.

## Notes

- The discover registry doesn't change mid-turn unless someone runs `easynet agent refresh`. Cache results within a single reply.
- For long-running missions (>30 s), use `device.mission.run` with a label, then `device.mission.track <run_id>` for status. The mission keeps running even if your turn returns first.
- "I want to BUILD a new ability instead of using one" is the `easynet-author` skill's domain, not this one.
