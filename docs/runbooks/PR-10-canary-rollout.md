# PR-10 production canary rollout runbook

**Purpose**: Operator-facing rollout playbook for the RFC-003
transport-plane production canary. Promotes the staging content
from `team-work/pr-drafts/PR-10-runbook-todo.md` to a stable
docs surface that operators can follow without spelunking
draft directories.

**Rollout shape**: one canary host, 24h soak, then per-host
roll with 1h gap between hosts. Per-host rollback path is
deterministic (binary backup → restore old unit → unpause).

**Pre-requisites**: PR-10 commits 1-4/N landed (TCP+TLS
listener, receipt store, receipt emission on accept + reject
+ Device URI-only). PR-2 / PR-7 ship'd. Trust anchor seeded
on the canary host (pairing flow has populated
`/etc/easynet/realm-trust.toml` for at least the backend
identity).

---

## step 1 — pre-flight checks (operator runs locally before touching prod)

- [ ] cert/key files present at the path `daemon-config.toml`
      points at, both readable by the daemon user, mode 0600
      (or owner-only equivalent on Windows).
- [ ] `realm-trust.toml` non-empty, contains the backend
      `01BAK` entry under `role = "backend"`.
- [ ] Replay store empty at boot (`SharedNonceReplayStore` is
      in-memory and empty by construction; no operator action
      unless a previous daemon left a process artifact).
- [ ] `cargo test --lib --features axon-pb services::axon_serve`
      green on the build that's about to ship.
- [ ] `go test -tags=e2e ./internal/daemon_grpc/...` green
      against the same build.
- [ ] Backup of the previous binary exists at
      `/usr/local/bin/easynet-daemon.bak` (deploy script
      writes this; verify mtime is the previous release).

## step 2 — canary host swap

```bash
ssh canary-host
sudo systemctl stop axon-runtime easynet-api    # legacy units
sleep 0.2                                        # wait-for-stop buffer
sudo cp /usr/local/bin/easynet-daemon{,.bak}     # back up current
sudo install -m0755 \
  ${RELEASE_DIR}/easynet-daemon /usr/local/bin/easynet-daemon
sudo systemctl daemon-reload
sudo systemctl enable --now easynet-daemon.service
journalctl -fu easynet-daemon.service            # confirm UDS bind +
                                                 # TCP+TLS bind logged
```

Confirm the daemon's stderr shows both:

```
[axon-serve] gRPC InvocationServer listening on UDS /var/lib/easynet/daemon.sock
[axon-serve] gRPC InvocationServer listening on TCP+TLS <addr> (cert=..., key=...)
```

If either bind fails: investigate, do NOT proceed. Boot is
fail-closed by PR-10 spec INV-1.

## step 3 — canary 24h soak

Monitor the **4 indicators** continuously for 24h:

1. **Backend error rate** vs. the 7-day baseline. Tolerance:
   ≤ 110% of baseline. Source: backend Prometheus
   `http_requests_total{status=~"5.."}` /
   `http_requests_total`.
2. **Device disconnect rate** (PresenceRegistry insert/remove
   churn). Tolerance: ≤ 120% of baseline. Source: daemon
   stderr grep `presence_registry` events / second.
3. **Latency p99**. Tolerance: ≤ 120% of baseline. Source:
   backend `http_request_duration_seconds_bucket` p99 vs.
   the 7-day p99.
4. **Unhandled errors in daemon logs**. Tolerance: 0
   `panicked at` / `gRPC UDS server exited with error`
   entries. Source: `journalctl -u easynet-daemon` filtered.

Receipt store sanity (PR-10 commits 2-4/N specific):

- [ ] `SharedReceiptStore` len visible via a future
      `/admin/receipts/recent` endpoint (RFC-N PR-N5
      territory). For PR-10 ship, the dev-tier check is
      grepping daemon stderr for the receipt-emit log lines.
- [ ] Receipt store size never exceeds
      `DEFAULT_RECEIPT_CAPACITY = 10_000`. (The store evicts
      FIFO; this is a property of the type, not an operator
      check.)

If all 4 indicators stay in tolerance for 24h: proceed to
step 4. If any go out of tolerance: step 6 (rollback).

## step 4 — per-host rollout (after canary clean)

For each remaining production host, sequentially with a 1h gap:

  a. **Stop existing units on this host**:
     ```bash
     systemctl stop axon-runtime easynet-api
     sleep 0.2
     ```

  b. **Install new binary, start single new unit**:
     ```bash
     systemctl daemon-reload
     systemctl start easynet-daemon
     ```

  c. **Wait 1h, monitor the same 4 indicators on this host**.

  d. **If clean → next host**. The 1h gap is the steady-state
     observation window.

  e. **If regression → rollback this host** (per step 6).
     Pause the rest of the rollout. Decide whether to revert
     the canary too:
       - canary also revert → roll the entire fleet back to
         PR-1..PR-9 stable; enter root-cause phase.
       - canary stays → reproduce in staging, fix, re-canary
         with the fixed build.

## step 5 — completion

After every host has soaked 1h clean: declare PR-10 ship'd.
Update `team-work/INDEX.md` with a final "PR-10 production
canary complete" entry citing the host count + final
indicator readings. Move
`pr-drafts/PR-10-spec-production-canary.md` to
`pr-drafts/_archive/` for historical reference.

## step 6 — rollback

Per-host:

```bash
systemctl stop easynet-daemon
sudo cp /usr/local/bin/easynet-daemon{.bak,}
systemctl daemon-reload
systemctl start axon-runtime easynet-api
```

The `.bak` was written in step 2; if it's missing,
something else has gone wrong — investigate before
attempting any further rollback. The `axon-runtime` unit
file is still on disk per PR-9's deferred SDK lifecycle
migration; restoring it is one `systemctl start`.

After fleet-wide rollback: file an incident, attach
`journalctl -u easynet-daemon` for the affected hosts +
the 4 indicators' chart shows for the soak window.

---

## What this runbook does NOT cover

- **Cert lifecycle automation** (renewal, ACME, etc.) —
  rotation today requires a daemon restart. A future
  watcher + `serve_with_shutdown` swap is RFC-N+ work.
- **Cross-hub federation rollout** — that's RFC-N PR-N1+
  per `team-work/letters/2026-05-01-49-haifeng-to-cto-mohao-liangbing-RFC-N-implement-master-plan.md`.
- **Receipt audit query operator surface** — the in-memory
  store is bounded to 10K; persistent / queryable audit is
  RFC-N PR-N5.

## References

- `team-work/pr-drafts/PR-10-spec-production-canary.md` —
  spec the implementation traces to
- `team-work/pr-drafts/PR-10-runbook-todo.md` — the original
  staging document this runbook supersedes
- RFC 001 §5.3 — admission-emits-receipt semantics (best-
  effort fail-open per PR-10 INV-5)
- DEC-012 — receipt deferral close (PR-10 commits 3-4/N)
