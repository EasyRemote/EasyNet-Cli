# AXON-RFC-001 plan v4.1.4 — URA six-role ontology

Status: ratified by CTO 2026-05-03
Supersedes §1 of plan v4.1.3

This plan revision corrects a load-bearing ontology error in v4.1.1
through v4.1.3: every Agent identity URI was collapsed under a
single `/agent/<ULID>` segment, conflating identity (user / hub),
host (device), and capability (ability) into one namespace. The
collapse made the ULID prefix (`01HUB`, `01BAK`, `01DEV-...`,
`01USR-...`, etc.) the load-bearing role discriminator — which is
fragile because (a) the prefix is opaque to admission, (b) every
new role required ULID prefix coordination, and (c) the URI parser
had to treat `01HUB.federation.advertise` as a structurally
ambiguous string.

v4.1.4 promotes the role distinction to a first-class URI segment.
There are exactly six role segments after the realm; the dot-to-
thing tail of each role has a deterministic structure the parser
recovers without prefix heuristics.

## §1 — URA grammar

```
easynet:///r/<realm>/<role>/<dot-to-thing>

realm  = hub DNS, e.g. "localhost" (dev) | "easynet.run" (prod)
role   ∈ { user, device, agent, ability, hub, resource }
```

| role     | dot-to-thing                                          | example                                                           |
|----------|-------------------------------------------------------|-------------------------------------------------------------------|
| user     | `<user-uuid>`                                         | `easynet:///r/easynet.run/user/5ff5ac67-ac43-...68ff`             |
| device   | `<device-uuid>`                                       | `easynet:///r/easynet.run/device/4065c47a-ec6f-...09b8`           |
| agent    | `<user-uuid>.<agent-id>`                              | `easynet:///r/easynet.run/agent/5ff5ac67-...68ff.claude`          |
| ability  | `<user-uuid>.<agent-id>.<ability-id>`                 | `easynet:///r/easynet.run/ability/5ff5ac67-...68ff.claude.fs.read`|
| hub      | (singleton — no tail)                                 | `easynet:///r/easynet.run/hub`                                    |
| resource | `<user-uuid>/<namespace>/<path>`                      | `easynet:///r/easynet.run/resource/5ff5ac67-.../fs/Users/x/y.md`  |

## §2 — Why these shapes

### 2.1 user-anchored ownership

Agent / ability / resource carry the owning user's UUID in the URI
because **ownership is a first-class property** of the entity, not
a runtime placement. When a user migrates from one Mac to another,
their `claude` agent and its `skill.alive-video` ability stay
identifiable; only the runtime placement (which device hosts them)
changes — and that placement lives in Directory metadata, not in
the URI.

The legacy `01LLM-<ulid>-<name>`, `01CON-<ulid>`, `01MCP-<ulid>`
prefixes retire — the new shape encodes ownership directly. A
user with multiple `claude` instances disambiguates via agent-id
suffixing (`claude-work`, `claude-personal`) at the agent level,
not via opaque ULID generation.

### 2.2 device is host, not agent

`device` is a physical host (the box running easynet-daemon). The
device → user binding lives in `device_pairings.user_id` (a NOT
NULL FK with ON DELETE CASCADE in the backend Postgres schema),
**not** in the URI. Encoding ownership in the device URI was always
redundant — the relational store is the source of truth.

device-id is a bare UUID. The legacy `en-<uuid>` prefix v1's
`createPairingLogic.go` minted retires. UUIDs already collide
~never; an extra prefix added size without meaning.

### 2.3 hub is a realm-singleton

Per realm, exactly one hub. v4.1.4 collapses the `01HUB` /
`01BAK` distinction — backend Go process IS the hub of its realm.
A multi-hub-per-realm topology (RFC-005 future work) would extend
hub URI with a tail; until then `easynet:///r/<realm>/hub` is
unambiguous and the parser strict-rejects any tail to catch v1
callers still emitting `/hub/01HUB`.

### 2.4 resource is user-anchored, slash-tailed

Resources (files, processes, PTY sessions, shells, HTTP endpoints)
belong to their owning user — same ontology as agent / ability —
and the hosting device is Directory metadata. Tail uses `/`
separator instead of `.` because path components inevitably
contain dots (filenames like `notes.md`); a dotted tail would
collide.

The middle segment is a typed namespace (`fs` / `process` / `pty`
/ `shell` / `http`), not a free-form catalog. The set is closed:
adding a new namespace requires landing a daemon-side handler AND
adding it to the parser's allowlist. Unknown namespaces are
strict-rejected.

### 2.5 ability tail is splitn(3, '.')

`<user>.<agent>.<ability-id>` — the first two dots are fixed
boundaries (user-id and agent-id are single tokens with no
internal dots). Everything after the second dot is the ability-id
verbatim and may itself contain dots: `fs.read`, `skill.alive-
video`, `fleet.list_agents`. This is deterministic — no registry
lookup, no namespace heuristic.

## §3 — AXIOM seven-tuple correspondence

The URA fills the caller / callee / subject slots:

| slot     | URA kind constraint                                                |
|----------|-------------------------------------------------------------------|
| caller   | hub / device / agent (the entity that signed and transmitted)     |
| callee   | hub / device / agent (the entity addressed)                       |
| subject  | device / resource (the operational principal — user is identity, |
|          | not action-target; in the JWT-delegation flow the subject is the |
|          | device acting for the user)                                       |

Admission's `ValidateSubject` enforces the kind constraint at
envelope dispatch. `ValidateSubject` lives in
`backend/internal/axon/admission.go` (Phase 5 landing — log-only
default; production toggles via env `EASYNET_ADMISSION_SUBJECT_ENFORCE`).

## §4 — Migration: v4.1.3 → v4.1.4

Phases (already shipped at the time this revision lands):

| Phase | Scope                                                          | Commit       |
|-------|----------------------------------------------------------------|--------------|
| 2A    | backend `axon/urns.go` 6-role builders + strict parser         | `58ecaf7`    |
| 2B    | backend callsite migration of deprecated wrappers              | `2b62f3a`    |
| 2C    | backend `Axon.Tenant` → `Axon.Realm` + retire `easynet-platform` | `f21bf60`  |
| 2D    | drop `en-` prefix on device-id; bare UUID                      | `9268d4b`    |
| 2E    | CLI Rust `src/uri.rs` 6-role builders + parser                 | `9502707`    |
| 2F    | CLI emitters (`easynet join`, `invoke --node`, federation_wire) | `bc98d92`+`242b6c6` |
| 2G    | dev-backend.sh `--reset-db` wipes trust-anchor + identity tree | `0ecbad9`    |
| 2H    | this spec doc (RFC-001 plan v4.1.4)                            | (this commit)|

Migration policy: **wipe-and-rejoin**. The dev-backend.sh script
clears every URA-bearing state file on `--reset-db`. Pairing is
one CLI command, and the dev seed re-creates the device row from
scratch on the next backend boot.

Production has no pre-v4.1.4 deploy with rows on disk yet (the
prod RealmEasyNet realm was never populated under the old
`Tenant: easynet-platform` value), so the same policy applies
trivially there.

## §5 — Parser strictness contract

Both backend Go (`backend/internal/axon/urns.go::ParseURA`) and
CLI Rust (`src/uri.rs::parse_ura`) implement the same strict
parser:

- `user`: tail must be a bare id with no dot, no slash.
- `device`: tail must be a bare id with no dot, no slash.
- `agent`: tail must be exactly `<user>.<agent>` with single dot,
  no slashes; agent-id MUST NOT contain a dot (that namespace
  belongs to ability/).
- `ability`: tail must be `<user>.<agent>.<ability>` with three
  non-empty parts via splitn(3, '.'); ability-id may contain dots
  preserved verbatim.
- `hub`: no tail (realm-singleton); presence of any tail is a
  strict reject.
- `resource`: tail must be `<user>/<namespace>/<path>` with at
  least one `/` separator; namespace must be in the allowlist
  (`fs`, `process`, `pty`, `shell`, `http`); unknown namespaces
  are strict-rejected.

The legacy v4.1.3 wide parser (accept any of six role segments
+ arbitrary tail) retires. Phase 1 (commit `433640f`) widened the
daemon parsers to accept any role segment as a forward-compat
bridge; Phase 2A landed the strict parser; this spec retires the
wide parser.

## §6 — Backwards compatibility

URIs minted by pre-v4.1.4 emitters do not parse under the new
strict parser. There are no backwards-compatibility shims in the
parser — that is by design. The wipe-and-rejoin policy in §4 makes
the parser strictness ergonomic: every URA on the wire is freshly
minted under the new shape.

The deprecated wrappers in `backend/internal/axon/uri.go`
(`BackendURI`, `OperatorURI`, `AgentIDFromURI`) are sweep targets
for Phase 4 — they exist only so the build stays clean while
callsites migrate. Their internals already emit v4.1.4 shapes;
removing them is a pure-rename Phase.

## §7 — Open questions deferred

- **Multi-hub per realm (RFC-005)**: would extend hub URI from
  singleton `easynet:///r/<r>/hub` to `easynet:///r/<r>/hub/<id>`.
  Parser changes minimal (allow tail when role == hub); naming
  ergonomics need ratify before landing.
- **Resource path versioning**: should `/resource/<u>/fs/path` ever
  carry an `@<version>` suffix the way `/abilities/<n>@<v>` did
  in v1's resource-URI shape? Unlikely — resources are content-
  addressable via their hosting substrate (filesystem mtime, git
  commit hash, etc.), not by URA versioning. Defer until a
  concrete use case appears.
- **Hub-owned ability resource URIs**: the existing
  `easynet:///r/prv/hub/<realm>/abilities/...` pattern in
  `runtime/advertise.rs` is a CALLEE resource URI (decorative
  from the daemon's perspective; routing happens via the
  `function_name` field). Whether to retire it in favor of a
  `/r/<realm>/hub/abilities/...` shape (no `<vis>` segment) is a
  Phase 3+ decision — not a v4.1.4 blocker.
