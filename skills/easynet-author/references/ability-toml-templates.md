# ability.toml templates

Four shapes. Each is a valid stand-alone manifest.

---

## 1. shell exec — argv subprocess

For wrapping CLIs (`curl`, `jq`, `awk`, scripts). Templated argv elements get rendered with `{{ name }}` placeholders against call args.

```toml
schema_version = "1"
name = "weather"
description = "Fetch the current weather for a city via wttr.in."

[input_schema]
type = "object"
required = ["location"]
additionalProperties = false
[input_schema.properties.location]
type = "string"
description = "City name, URL-safe."

[exec]
kind = "shell"
sandbox = "net_only"      # one of: none | net_only | pure_compute
argv = ["curl", "-s", "https://wttr.in/{{ location }}?format=3"]
# stdout = "utf8_trim"     # optional decoder; default is utf8_trim
```

Sandbox profiles:

- `none` — no sandbox (default). Use for trusted local tools (`hostname`, `df`).
- `net_only` — deny non-`/tmp` writes; allow outbound network. Right for `curl`-style.
- `pure_compute` — deny network and non-`/tmp` writes. Right for `jq`/`awk` filters.

Argv values pass without `sh -c`, so a value containing whitespace can't expand into multiple tokens — that's the safety guarantee.

---

## 2. http exec — one in-process HTTP call

Lighter than shell + curl. URL placeholders are URL-encoded automatically.

```toml
schema_version = "1"
name = "weather_http"
description = "Fetch weather over HTTP without spawning curl."

[input_schema]
type = "object"
required = ["location"]
[input_schema.properties.location]
type = "string"

[exec]
kind = "http"
method = "GET"
url    = "https://wttr.in/{{ location }}?format=3"
# headers = { "Accept" = "text/plain" }    # optional
# body    = "..."                          # optional, for POST/PUT
# response = "text_trim"                   # decoder; default text_trim
```

Schemes restricted to `http` / `https`. CR/LF in header values is rejected. Response body is capped at 1 MiB; bigger responses error rather than silently truncate.

---

## 3. eal exec — compose existing abilities into a workflow

The body is an EAL `mission "x" { … }` program. References to `<agent>.<verb>` must exist in this owner's catalog (the `validate_authored_ability` check enforces this).

```toml
schema_version = "1"
name = "trip_briefing"
description = "Combine weather + a quip into a single trip-briefing reply."

[input_schema]
type = "object"
required = ["location"]
[input_schema.properties.location]
type = "string"

[exec]
kind = "eal"
result_binding = "briefing"
source = """
mission "trip-briefing" {
  let w = claude.weather(location: "{{ location }}")
  let q = codex.quip(topic: "weather in {{ location }}")
  let briefing = claude.compose(parts: [w.output, q.output])
}
"""
```

`{{ location }}` is rendered against call args BEFORE the EAL parser runs. Missing args fail with a clear error attributed to "eal executor".

`result_binding` (optional) names which `let`-bound value becomes the ability's `result` field. Without it, the entire `bound_vars` map is returned.

---

## 4. chat fallback — LLM-fulfilled

No `[exec]` block. The agent's chat handler is invoked with a synthesized prompt: "fulfill ability `<verb>` with these args". Use when there is no deterministic recipe and you trust the LLM to interpret.

```toml
schema_version = "1"
name = "summarise_complaints"
description = "Summarise customer complaint threads into a one-paragraph status."

[input_schema]
type = "object"
required = ["thread"]
[input_schema.properties.thread]
type = "string"
description = "Raw transcript of the complaint thread."
```

That's the entire manifest. The dispatcher routes `claude.summarise_complaints` to `claude.chat` with the thread embedded.

---

## Field reference

| Field | Required | Notes |
|---|---|---|
| `schema_version` | yes | `"1"` today. Wrong value rejected at parse. |
| `name` | yes | matches the file stem (`weather.ability.toml` ⇔ `name = "weather"`). |
| `description` | yes | one-line. |
| `[input_schema]` | yes | JSON Schema subset. `type = "object"` mandatory. |
| `[output_schema]` | no | optional — pin a typed return contract. Chat-style abilities omit. |
| `[exec]` | no | absence = chat-fallback. |
| `[exec].kind` | when `[exec]` set | one of `shell` / `http` / `eal`. |
| `[exec].timeout_seconds` | no | per-call deadline. Defaults to 3600 s for the EAL exec, 30 s for shell/http. |
| `[access].visibility` | no | `selfish` / `device` / `public`. Default `device`. |

---

## Don't

- ❌ Hardcode credentials in `argv` or `body`. Read from env or pass via args.
- ❌ Use `kind = "shell"` for HTTP — use `kind = "http"`. The dedicated executor skips the subprocess spawn (~50 ms).
- ❌ Reference `<agent>.<verb>` in EAL exec when the verb isn't in the owner's catalog. The validator catches it; publishing manually would land a dead ability.
- ❌ Set `name = "chat"`. Reserved.
