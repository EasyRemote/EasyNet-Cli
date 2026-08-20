# Desktop Usage

This guide is for running the checkboard from a developer desktop, including
Codex desktop and Docker Desktop on macOS.

## Preconditions

- Docker Desktop is running before Docker-tagged tests start.
- The current repository is `EasyNet-Cli`.
- The sibling EasyNet repository exists at `../EasyNet` when external Docker
  harnesses are selected.
- Local images exist or the selected command builds them:
  - `easynet/hub-e2e:local`
  - `easynet/device-e2e:local`
- Ports used by the three-node Docker topology are free:
  - `18080` for Hub HTTP
  - `50443` for Hub daemon TLS

Check Docker before running Docker tests:

```bash
docker info
docker images | grep 'easynet/.\\+-e2e'
```

## Recommended Desktop Flow

1. Open the EasyNet-Cli workspace in Codex desktop.
2. Run a syntax check for the checkboard runner:

   ```bash
   bash -n packaging/checkboard/run-checkboard.sh
   ```

3. Run the default non-external inventory:

   ```bash
   packaging/checkboard/run-checkboard.sh
   ```

4. Run the three-node Docker user scenario:

   ```bash
   packaging/checkboard/run-checkboard.sh --filter docker-three-node-cli-real-user --include-docker
   ```

5. Open the generated `report.md` path printed at the end.

## When A Test Fails

Start from the case directory listed in the summary. The fastest path is:

```bash
cat target/e2e/checkboard/<run-id>/<case-id>/error.md
tail -120 target/e2e/checkboard/<run-id>/<case-id>/stderr.log
tail -120 target/e2e/checkboard/<run-id>/<case-id>/stdout.log
```

For Docker topology failures, also inspect the test's own generated report
under `target/e2e/docker-three-node-cli-real-user/` or the sibling EasyNet
script output when the command delegates into `../EasyNet`.

## Known Desktop-Specific Signals

- `Docker engine is not running or the Docker socket is unavailable`: Docker
  Desktop is closed or still starting.
- Devices visible as `UNKNOWN` in `easynet auth devices --json`: the backend
  device row exists, but the Hub session projection has not completed. In the
  current three-node test this exposed a Hub user trust bootstrap gap:
  `federation.resolve_key` cannot resolve the paired user URA in the Hub trust
  set.
- `runtime start` / `runtime stop` are not run by default inside the Docker
  device containers because the container entrypoint supervises the daemon.
