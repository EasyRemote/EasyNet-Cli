# EAL grammar reference

EAL (EasyNet Ability Language) is a small DSL for orchestrating ability calls. The compiler infers the dependency DAG from variable references and runs independent steps in parallel.

## Minimal program

```eal
mission "name" {
  let r = call "echo" on "my-device" with {
    message = "Hello from EAL!"
  }
}
```

The outermost `mission "name" { … }` is required. The empty form `mission "x" {}` is legal (used as a sentinel by some control-flow paths).

## Two call surfaces

### Member-call form (preferred for agent abilities)

```eal
let r = claude.weather(location: "Beijing")
```

This dispatches to the agent named `claude`'s `weather` ability. The dispatcher routes it through the daemon's ability registry — same path `easynet ability invoke claude.weather` takes.

### Traditional form (device-only)

```eal
let r = call "weather" on "device-01" with { location = "Beijing" }
```

This dispatches to a registered DEVICE node — not an agent. The EAL surface invariant: traditional form is strictly device-only. If `device-01` collides with a registered agent, the parser rejects the program with a "use member-call form" hint.

## Statement forms

```eal
mission "example" {
  // Bind to a variable for downstream reference
  let result = claude.summarise(text: "raw input")

  // Fire-and-forget (no binding)
  call "notify" on "speaker" with { sound = "ping" }

  // Reference a prior step's output (auto-builds the DAG)
  let summary = claude.summarise(text: "more text")
  let saved   = device.save(content: summary.output)

  // Archive important mission values for downstream answer/report stages
  emit "summary" kind answer value summary.output
  emit "chain" kind context value "summarise -> save"
}
```

`summary.output` carries `summarise`'s result into `save`. The compiler sees the dependency and runs `save` only after `summarise` finishes. Steps with no shared dependencies execute in parallel.

## Emit statements

```eal
emit "terminal_rows" kind answer value rows.output
emit "note" kind diagnostic value "fallback path"
```

`emit` appends an ordered archive record to the mission trace. It is not an
ability call, does not dispatch, and does not change step phases. Emitted names
do not need to be unique; order is preserved by `seq`. Top-level `emit` can
reference a prior binding or a literal value. Loop-local emit is not supported
in this EAL version.

## Step options

```eal
let r = claude.weather(location: "Beijing")
  timeout 30
  retries 2
  on_failure retry
```

| option | values | meaning |
|---|---|---|
| `timeout <secs>` | integer | per-step deadline |
| `retries <n>` | integer | attempts before declaring failure |
| `on_failure <policy>` | `abort` / `skip` / `retry` / `continue` | what to do if the step fails. `continue` (default) records the failure but lets dependents see `UpstreamFailed` |
| `optional` | (flag) | step may fail; the mission keeps going regardless |

## Loops

```eal
mission "polling" {
  loop "stable-output" max_iters: 5 {
    body {
      let r = device.read_sensor()
    }
    verify {
      let ok = device.is_settled(reading: r.output)
    }
  }
  let final = use(stable-output.result)
}
```

Named loop exports `<name>.result` at the enclosing scope; anonymous loops don't.

## Templating in EAL exec abilities

When EAL is the body of an `[exec] kind = "eal"` ability, the source goes through `{{ name }}` templating BEFORE parse:

```eal
mission "wrap" {
  let r = claude.weather(location: "{{ city }}")
}
```

Call args fill the placeholders. Missing args fail with a clear error attributed to "eal executor", before the EAL parser sees a half-rendered program.

Inside string literals the templating still runs — `"{{ x }}"` becomes the value of arg `x` (string values insert raw, other JSON values insert as `to_string()`).

## What EAL is NOT

- Not a Turing-complete scripting language. No conditionals (yet — the loop block is the only control-flow construct), no functions, no closures.
- Not an MCP tool wrapper. EAL programs run through the EasyNet daemon's dispatcher, which calls into ability handlers (which may themselves wrap MCP tools).
- Not real-time. Step ordering follows the planner's phases; intra-phase parallelism is up to the dispatcher.

## Compile + run

```bash
# Parse + plan (no execution)
easynet mission compile path/to/x.eal --emit-ir

# Execute
easynet mission run path/to/x.eal

# Or programmatically via easynet.run inside an MCP context
easynet ability invoke easynet.run --args '{"source": "<EAL text>", "label": "x"}'
```

A run produces a directory under `~/.easynet/missions/runs/<id>/` containing `source.eal`, `ir.json`, per-step receipts, and final `meta.json`. `easynet mission show <id>` reads them back.

## Common patterns

### Two-step pipeline

```eal
mission "fetch-summarise" {
  let raw = claude.fetch_url(url: "https://example.com")
  let s   = claude.summarise(text: raw.output)
}
```

### Fan-out + synthesise

```eal
mission "compare" {
  let a = claude.argue(position: "for")
  let b = codex.argue(position: "against")
  let synthesis = claude.synthesise(views: [a.output, b.output])
}
```

`a` and `b` run in parallel (no mutual deps). `synthesis` waits for both.

### Optional step

```eal
mission "best-effort" {
  let main = claude.do_thing()
  call "notify" on "speaker" with { sound = "done" } optional
}
```

The notify step can fail without taking the mission down.
