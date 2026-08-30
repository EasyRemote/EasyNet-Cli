# Decisions and evidence

- Decision: the two-node smoke's manually-authored native EasyNet manifest now
  includes `"protocol": "binary_v1"` under `exec`.
- Reason: EasyRemote's resident `HostServer` speaks the binary ERHS frame
  protocol. Omitting the protocol made the daemon select the legacy JSON-lines
  host_stream reader, which surfaced as `STREAM_TRUNCATED: read frame:
  Connection reset by peer`.
- Evidence update: run output directory
  `target/e2e/docker-two-node-easyremote-cli/20260823-035752/` proves the
  native EasyNet `handle_call`, `handle_stream`, canonical URA call/stream,
  typed stub, and typed stub stream assertions all passed after adding
  `binary_v1`.
- Follow-up finding: externally uninstalling `er.add` while the EasyRemote
  provider process remains alive is the wrong lifecycle assertion. EasyRemote
  owns a binding lease and renews desired live abilities; a live provider may
  legitimately redeploy the ability after a manual uninstall.
- Follow-up decision: the smoke now stops the EasyRemote provider lifecycle,
  lets `ComputeNode.stop()` revoke all EasyRemote deployments, and waits until
  the caller no longer sees `er.add`, `er.total`, `er.merge`, `er.defaulted`,
  `er.summarize`, `er.bundle`, `er.countdown`, or `er.whoami`.
- Manual `ability.uninstall` remains covered by the independently CLI-deployed
  native EasyNet `nativeer.native_echo` ability, which has no EasyRemote renewal
  owner.
- Final evidence: `bash tools/scripts/docker-two-node-easyremote-cli-e2e.sh
  --skip-build --project easynet-easyremote-two-node-final --out-dir
  target/e2e/docker-two-node-easyremote-cli/20260823-lifecycle-stop-final`
  passed. `report.json` has no false assertions and specifically proves:
  native handle call/stream, native canonical URA call/stream, native typed
  stub call/stream, six native invocation records with receipt chains,
  EasyRemote provider stop removes all EasyRemote abilities from caller-visible
  catalog, and native manual uninstall removes the CLI-deployed native ability.
