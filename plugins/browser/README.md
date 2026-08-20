# EasyNet Browser Plugin

`easynet.browser` is the package-owned browser executor for EasyNet. It opens a
real Chrome/Chromium target and exposes human-scale input, viewport capture, and
raw target-scoped Chrome DevTools Protocol (CDP) over governed Axon abilities.

## Runtime model

- Headed Chrome is the default. A human can see and operate the same page that
  an agent controls; `headless = true` is explicit.
- Every launched browser uses an isolated, non-default user-data directory.
  This is required by current Chrome remote-debugging policy and prevents the
  plugin from attaching debugging privileges to the human's default profile.
- The resolver chooses the numerically newest installed Stable candidate from
  system Chrome/Chromium and EasyNet-owned Chrome for Testing installations under
  `~/.easynet/browser/chrome/<version>`. `EASYNET_BROWSER_CHROME` or the
  `executable_path` open argument may select an exact executable. The owned
  root can be changed consistently with `EASYNET_BROWSER_CHROME_ROOT`.
- CDP is negotiated at runtime with `Browser.getVersion`; the plugin uses raw
  JSON commands instead of pinning generated bindings to one browser release.
- Existing CDP endpoints must be loopback HTTP or WebSocket endpoints. Chrome
  processes launched by the plugin are owned and terminated by the session;
  explicitly connected processes are never terminated.

The official current Chrome for Testing release and downloads are published in
the [Chrome for Testing availability dashboard](https://googlechromelabs.github.io/chrome-for-testing/).
Chrome's remote-debugging security requirements are documented in the
[Chrome remote debugging policy update](https://developer.chrome.com/blog/remote-debugging-port).

Install or update the plugin-owned browser from the official Stable channel:

```bash
bash plugins/browser/tools/install-current-chrome-for-testing.sh
```

The installer reads the current version and platform URL from the official
Chrome for Testing metadata at runtime, verifies the extracted executable's
reported version, and atomically promotes it into the EasyNet-owned browser
root. It never modifies a system Chrome installation or an existing version.

## Ability surface

| Ability | Axon mode | Purpose |
| --- | --- | --- |
| `browser.open_session` | RPC | Open a governed page and return its resource URA. |
| `browser.show_session` | RPC | Read redacted lifecycle and protocol state. |
| `browser.send_input` | RPC | Apply navigation, pointer, keyboard, text, or fill input. |
| `browser.capture_viewport` | Stream | Emit a finite bounded sequence of real CDP screencast frames. |
| `browser.attach_session` | InvokeBidi | Carry CDP commands, responses, events, and input JSON frames. |
| `browser.close_session` | RPC | Idempotently close the target and owned process. |

After open, the returned resource URA is the invocation subject for every other
operation. The creator identity and resource subject are both checked by the
plugin. Axon remains responsible for admission, routing, stream/bidi lifecycle,
receipts, and terminal semantics; the plugin does not expose a public browser
WebSocket or JSON-control route.

`browser.attach_session` accepts one `cdp.command` per frame for latency-sensitive
interactive work and `cdp.batch` with 1–32 commands for throughput-sensitive DOM
inspection or automation. Batch command IDs must be unique. The response is one
`cdp.batch_response` carrying the same batch ID and an ordered array of ordinary
`cdp.response` objects. All raw commands share 32 permits, so concurrent batches
cannot bypass the pending-call bound. High-level `input` frames use a separate
arrival-ordered lane; when the fixed operation window is full, Axon backpressure
propagates to the caller.

Application fields are bounded below the Axon frame limit: CDP method and
correlation strings are at most 256 bytes, selectors 4 KiB, and URLs, text,
values, endpoint strings, and executable paths 64 KiB. The checked-in ability
schemas publish the corresponding `maxLength` constraints; runtime validation
remains authoritative for UTF-8 byte limits and raw CDP frames.

Session lifecycle is explicit: active rows move to a separate closing set before
external teardown. Closing rows still consume session/profile capacity and are
available only to idempotent `close_session`; every other operation fails closed.
Concurrent closes wait for the same terminal state. A target detach or CDP
connection loss triggers the same runtime-owned close path.

```json
{
  "type": "cdp.batch",
  "id": "inspect-1",
  "commands": [
    {"id": 1, "method": "DOM.getDocument"},
    {"id": 2, "method": "Runtime.evaluate", "params": {"expression": "document.title", "returnByValue": true}}
  ]
}
```

## Verification

```bash
cargo test -p easynet daemon::plugins::browser --lib
cargo test -p easynet current_chrome_real_cdp_smoke --lib -- --ignored --nocapture
cargo test -p easynet current_chrome_axon_bidi_performance --lib -- --ignored --nocapture --test-threads=1
bash tools/scripts/check-browser-cdp-axon-boundary.sh
```

The real-browser smoke test launches the newest candidate in headless mode,
attaches with `Target.attachToTarget(flatten=true)`, evaluates the document title
on the target session, and then tears down the target and owned process.
The performance probe additionally binds the provider into the real Axon
LocalRuntime adapters and emits one `BROWSER_CDP_AXON_METRICS=<json>` line with
interactive P50/P95/P99, batched throughput, first viewport frame, open, attach,
and close timing.
