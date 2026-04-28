# Plan v4.1.1: Theory of Operation — Post-RFC-001 EasyNet (Binding Contract)

## Why this revision

v4.1 was conditionally approved. v4.1.1 patches 9 remaining contract
gaps and inlines the previously-omitted §19 mapping table. After v4.1.1,
the document is binding: implementation may start.

The 9 patches:

| # | Topic | Section |
|---|---|---|
| P1 | Hosted child Agent identity + receipt signing | §A12, §1.3 |
| P2 | Persistent hosted Agent URI allocation | §A13, §1.4 |
| P3 | join_receipt_hash is membership lineage root, not global causal root | §A8 (revised), §3 Step E |
| P4 | admission_internal=true is kernel-local, non-user-settable | §A6 (revised), §14 |
| P5 | DelegationProof schema | §A14, §1.5 |
| P6 | Single mcp-profile Agent owning both bridge and client | §1, §13, §18 |
| P7 | §19 mapping table inlined in full | §19 (new) |
| P8 | conversation.* default visibility = SCOPED | §18 |
| P9 | AbilityDescriptor scope metadata schema | §A15, §1.6 |

---

## §0 — Architectural axiom (final)

```
A1.  The only network primitive is Invocation.
     invoke(caller, callee, ability, args, causal_context, nonce, subject) → receipt

A2.  The only ontology types are Agent + Ability + Invocation + Receipt.
     Agents have no kind, no role, no type discriminator at the protocol
     level. Implementation profiles describe ability bundles only.

A3.  Node-to-node communication MUST be Invocation.
     MCP / A2A / OpenAI tool_use exist ONLY at edge-adapter Agents.
     Backend, daemon, hub are all "nodes". Frontend and external MCP
     clients are "applications".

A4.  Every Invocation envelope distinguishes:
       caller       — the Agent that signed and transmitted the envelope
       subject      — the principal whose authority the call invokes
       delegation   — DelegationProof (§1.5) when subject ≠ caller
     A node MUST sign with its own key as caller. Subject is asserted
     via DelegationProof. Admission gate validates both caller signature
     and delegation chain.

A5.  Identity has two phases:
       provisional  — pre-membership; URA = "provisional:<pubkey-fingerprint>"
                      only valid as caller of federation.join
       canonical    — post-membership; URA = easynet:///r/<realm>/agent/<ulid>
                      assigned by hub in federation.join receipt
                      MUST be used in all subsequent envelopes
```

---

## §1 — Population: who exists

```
realm: acme

hub-profile Agent  (federation directory + trust anchor)
  uri:               easynet:///r/acme/agent/01HUB
  signing authority: own keypair (trust root)
  abilities:         federation.{join, leave, resolve, advertise_agent,
                                 advertise_abilities, heartbeat, revoke, federate}
                     identity.{rotate_key, revoke_key, issue_token, ...}
                     policy.{evaluate, simulate, publish, list}        (if also runs policy)
                     transport.relay.{invoke, stream}                   (optional)
  process:           embedded in any easynet-daemon with profiles.hub=true

device-profile Agent  (one per host running easynet-daemon)
  uri:               easynet:///r/acme/agent/01DEV-<ulid>  (canonical, persisted)
  signing authority: own keypair (the host's daemon key)
  abilities:         fleet.{list_agents, list_abilities, list_sessions,
                            start_agent, stop_agent, attach_session,
                            session_input, session_read, session_resize, session_close}
                     admin.{failover, snapshot, recover, status}
                     observe.{health, network_health}
                     meta.{describe, list_abilities}
  process:           every easynet-daemon advertises one device-profile Agent

consent-profile Agent
  uri:               easynet:///r/acme/agent/01CON-<ulid>  (canonical, persisted)
  signing authority: hosted by device-profile (§1.3)
  abilities:         consent.{request, subscribe, decide, list_pending}
  process:           co-located in easynet-daemon when profiles.consent=true

policy-profile Agent
  uri:               easynet:///r/acme/agent/01POL-<ulid>  (canonical, persisted)
  signing authority: hosted by device-profile (§1.3) OR by hub-profile if co-located
  abilities:         policy.{evaluate, simulate, publish, list}
  process:           co-located when profiles.policy=true

mcp-profile Agent  (single Agent for both MCP edge ingress AND egress) [P6]
  uri:               easynet:///r/acme/agent/01MCP-<ulid>  (canonical, persisted)
  signing authority: hosted by device-profile (§1.3)
  abilities:         mcp.bridge.{list_tools, call_tool}     (incoming MCP, when profiles.mcp_bridge=true)
                     mcp.client.{list, call}                 (outgoing MCP, when profiles.mcp_client=true)
  process:           co-located when EITHER profiles.mcp_bridge OR profiles.mcp_client is true.
                     Hosts MCP server on ~/.easynet/mcp.sock for ingress.
                     Holds outbound MCP client connections for egress.
                     The two ability groups can be enabled independently
                     but live in one Agent identity.

llm-profile Agent  (one per registered AI like claude / codex)
  uri:               easynet:///r/acme/agent/01LLM-<ulid>-<name>  (canonical, persisted)
  signing authority: hosted by device-profile (§1.3)
  abilities:         conversation.{send, stream}                (default SCOPED [P8])
                     session.{create, list, resume, close}
                     meta.{describe, list_abilities, acquire, forget, publish, compose, cancel}
                     <skills as PRIVATE abilities>
  process:           spawned by device-profile.fleet.start_agent

backend-profile Agent  (the EasyNet web platform)
  uri:               easynet:///r/acme/agent/01BAK
  signing authority: own keypair (the backend service key)
  abilities:         aggregate.{list_skills_across_fleet, list_abilities_catalog}
  process:           backend Go service; embeds axon SDK; advertises self at boot

human  (operator)
  Not a daemon-resident Agent. Materialized only as a subject in
  Invocation envelopes that backend-profile (or device-profile, for
  local CLI) originates on the operator's behalf, with DelegationProof.
  uri shape:         easynet:///r/acme/agent/01USR-<user-id>
                     (URA exists for audit; no daemon hosts it)
```

### Configuration: `[profiles]`, no `[roles]`

```toml
# ~/.easynet/config.toml
[profiles]
device     = true     # always
hub        = false    # opt-in
consent    = true     # default for interactive hosts
policy     = false    # opt-in
mcp_bridge = true     # incoming MCP socket; default
mcp_client = true     # outgoing MCP; default

# Note: profiles.mcp_bridge and profiles.mcp_client both contribute
# abilities to the SAME mcp-profile Agent identity (§1, [P6]).
```

### §1.1 — "Skill" maps to "PRIVATE Ability of an LLM Agent"

```
~/.claude/skills/<dir>/SKILL.md   →  PRIVATE Ability of the claude llm-profile Agent
~/.agents/skills/<dir>/SKILL.md   →  PRIVATE Ability of the codex llm-profile Agent
<agent-root>/skills/<dir>/        →  same, for whichever llm Agent owns it

Each becomes:
  name:         skill.<directory_name>
  visibility:   PRIVATE
  scope:        owner_agent_uri = the LLM sub-agent's URA
  schema:       synthesized from SKILL.md frontmatter
```

### §1.2 — Two catalogs, not one

```
LocalAgentCatalog   (per daemon, in-memory)
  Map: agent_uri → LocalDispatchBinding
    where LocalDispatchBinding = InProcess(handler) | LocalIPC(socket) | LocalChild(pid)
  Built at boot from profiles + managed sub-agents + persisted ~/.easynet/local-agents.json.
  Holds dispatch handles for Agents physically resident on this host.
  Never sent over the network.

RealmDirectory      (held by hub-profile Agent; cached by every daemon)
  Map: agent_uri → DirectoryEntry
    where DirectoryEntry = {
      identity: { public_key, signing_authority, host_attestation? },
      abilities: AbilityDescriptor[],     // §1.6
      endpoints[],
      metadata,
      status                              // active | revoked | suspended
    }
  Replicated via federation.heartbeat deltas; persisted on hub.
  Never includes dispatch handles.
```

A daemon dispatching an Invocation:
1. Look up callee_uri in LocalAgentCatalog. Hit → in-process / IPC / child dispatch. Done.
2. Miss → look up callee_uri in cached RealmDirectory. Get endpoints. gRPC Invoke to remote daemon.
3. Cache miss → invoke `federation.resolve` on hub. Update local cache. Then retry step 2.

### §1.3 — Hosted Agent identity and signing authority [P1]

EasyNet supports two Agent identity models. Both are valid; the choice
is per-Agent and recorded in DirectoryEntry.identity.signing_authority.

**Model A — Self-signing Agent (own keypair).**
- The Agent owns its private key.
- Receipts the Agent issues are signed with its own key.
- Used by: hub-profile, device-profile, backend-profile.
- DirectoryEntry.identity.signing_authority = "self".

**Model B — Hosted Agent (signed by host).**
- The Agent has no private key of its own.
- Receipts the Agent issues are signed by the **hosting device-profile
  Agent**, with an attestation header indicating "this receipt was
  produced by hosted Agent X under host Y."
- Used by: consent-profile, policy-profile, mcp-profile, llm-profile,
  and any sub-agent spawned by `fleet.start_agent`.
- DirectoryEntry.identity.signing_authority = "hosted_by:<host_device_agent_uri>".
- Receipt schema for hosted Agent's response includes:
  - `callee_agent_uri` — the hosted Agent (the apparent callee)
  - `signer_agent_uri` — the hosting device-profile Agent
  - `host_attestation` — signed assertion that signer hosts callee

A receipt verifier checks: signer_agent_uri's key signs the receipt;
the host_attestation in DirectoryEntry confirms signer hosts callee.

**Model B is the default for daemon-spawned local Agents.** It avoids
key proliferation and matches the operational reality that the daemon
process is the sole signing authority for everything it spawns. Model
A is reserved for Agents whose identity must outlive the host (hubs,
backends).

### §1.4 — Persistent hosted Agent URI registry [P2]

```
Path:    ~/.easynet/local-agents.json   (mode 0600)
Owner:   the easynet-daemon process
Format:
  {
    "host_device_agent_uri": "easynet:///r/acme/agent/01DEV-...",
    "hosted_agents": [
      {
        "profile":            "consent",
        "name":               "default",
        "agent_uri":          "easynet:///r/acme/agent/01CON-...",
        "signing_authority":  "hosted_by:easynet:///r/acme/agent/01DEV-...",
        "first_seen_at":      "<iso-timestamp>"
      },
      {
        "profile":            "mcp",
        "name":               "default",
        "agent_uri":          "easynet:///r/acme/agent/01MCP-...",
        "signing_authority":  "hosted_by:easynet:///r/acme/agent/01DEV-..."
      },
      {
        "profile":            "llm",
        "name":               "claude",
        "agent_uri":          "easynet:///r/acme/agent/01LLM-...-claude",
        "signing_authority":  "hosted_by:easynet:///r/acme/agent/01DEV-..."
      }
    ]
  }
```

Rules:
1. The device-profile Agent mints hosted Agent URIs (ULIDs) for any
   Agent it spawns or instantiates locally.
2. Once minted, the URI is persisted to local-agents.json and reused
   across daemon restarts. Daemon never regenerates a hosted URI for
   the same `(profile, name)` pair.
3. Hub accepts hosted Agent URIs only via `federation.advertise_agent`
   from the hosting device-profile, with the hosting device's
   signature attesting host_attestation.
4. If local-agents.json is lost (disk failure), the device-profile
   re-mints fresh URIs and advertises them as new Agents; hub may
   garbage-collect stale entries via TTL on heartbeat absence.

### §1.5 — DelegationProof schema [P5]

When an Invocation's `caller` differs from its `subject`, the envelope
MUST carry a DelegationProof:

```
DelegationProof {
  issuer_uri          // who issued this delegation (e.g. backend, identity-issuer)
  subject_uri         // the principal being represented
  caller_uri          // the Agent authorized to act for subject (must equal envelope.caller)
  audience            // restriction: which callee(s) this delegation may target
                      //   may be specific URI, URI prefix, or "*" for unrestricted
  scopes[]            // ability-name patterns this delegation allows
                      //   e.g. ["federation.resolve", "fleet.list_*", "conversation.send"]
  issued_at
  expires_at
  signature           // signed by issuer's key
}
```

Verification rules at the admission gate:

1. **Signature integrity**: signature verifies against issuer_uri's
   public key (looked up in RealmDirectory).
2. **Issuer trust**:
   - If `issuer_uri == subject_uri`: any subject may issue delegation
     for themselves. Verify against subject's public key (recorded at
     federation.join or during key rotation).
   - Else: issuer_uri must be in the realm's "trusted delegation
     issuers" list (configured at hub; e.g. backend-profile may be
     trusted to issue delegations for human operators it has
     authenticated via JWT).
3. **Caller match**: caller_uri MUST equal envelope.caller. Otherwise
   reject — caller cannot reuse another Agent's delegation.
4. **Audience match**: envelope.callee MUST satisfy audience pattern.
5. **Scope match**: envelope.ability MUST satisfy at least one scope
   pattern.
6. **Expiry**: now < expires_at.

If any check fails, admission gate returns `Receipt { Failed,
DELEGATION_INVALID, reason }`. Subject is treated as the caller for
audit purposes when delegation is missing (envelope.subject defaults
to envelope.caller).

JWTs from web auth flows can be wrapped in DelegationProof: backend
authenticates a user via JWT, then constructs a DelegationProof with
issuer_uri = backend-profile's URA, subject_uri = the user's URA,
caller_uri = backend-profile's URA, audience = whichever hub or device
the call targets, scopes = the JWT's scope claims, signed by the
backend's key.

### §1.6 — AbilityDescriptor schema [P9]

Every advertised Ability is described by:

```
AbilityDescriptor {
  name                  // e.g. "fleet.list_abilities", "skill.alive-video"
  owner_agent_uri       // the Agent that hosts this Ability (always callee in invoke)
  visibility            // PRIVATE | SCOPED | PUBLIC
  scope_subjects[]      // when SCOPED: list of subject URAs (or URA prefixes) allowed to invoke
  scope_agents[]        // when SCOPED: list of caller URAs allowed (in addition to scope_subjects check)
                        //   both lists are checked; subject must match scope_subjects (or be empty/unrestricted)
                        //   AND caller must match scope_agents (or be empty/unrestricted)
  source                // free-form provenance, e.g. "skill_md:~/.claude/skills/alive-video/SKILL.md"
                        //   or "manifest:<agent-root>/abilities/foo.ability.toml"
                        //   or "kernel:built-in"
  schema_summary {
    input               // JSON Schema (or summary thereof for catalog listings)
    output_receipt_body // JSON Schema (or summary)
  }
  hints {
    read_only           // bool
    destructive         // bool
    idempotent          // bool
    streaming_only      // bool
  }
}
```

Visibility filter at `federation.resolve` and `meta.list_abilities`:

```
PUBLIC      → always returned
SCOPED      → returned iff (subject ∈ scope_subjects OR scope_subjects empty)
                       AND (caller ∈ scope_agents   OR scope_agents   empty)
              with empty meaning "no restriction on that axis"
PRIVATE     → returned iff caller is the owner_agent_uri's signing authority
                          (i.e. the device-profile that hosts the owning Agent)
              OR subject is the operator who owns the host
              Otherwise omitted from response
```

PRIVATE Abilities are functionally a degenerate SCOPED case: scope =
{the host's own operator}. This unifies the model.

---

## §2 — Daemon startup

### §2.A — Joined-device startup (common case)

```
Step 1.  Daemon process starts.
         Reads ~/.easynet/config.toml. Sees profiles.device = true.
         Reads ~/.easynet/credentials.json. MUST exist for this mode.
           credentials = { realm, canonical_agent_uri, public_key,
                           membership_token, hub_endpoints,
                           join_receipt_hash }
         If missing AND profiles.hub = false: daemon prints "run
         easynet join <token>"; exits.

Step 2.  Embedded axon kernel initializes.
         Allocates LocalAgentCatalog and the cached RealmDirectory.

Step 3.  Daemon reads ~/.easynet/local-agents.json (§1.4). For each
         hosted Agent recorded there with a profile that is enabled in
         config: instantiate the Agent with the recorded canonical
         URA, register in LocalAgentCatalog with InProcess binding.

         For each profile that is enabled but has no entry in
         local-agents.json yet: device-profile mints a fresh URA,
         appends to local-agents.json, instantiates, registers.

         Profiles processed: device (always), consent, policy, mcp,
         and per-managed-sub-agent llm.

Step 4.  For each managed sub-agent in ~/.easynet/agents.json (claude,
         codex, ...): same as Step 3 with profile=llm and name=<sub-agent name>.
         Scan ~/.claude/skills (or equivalent) and synthesize PRIVATE
         AbilityDescriptors for each skill directory. Owner is the LLM
         Agent's URA; scope = host operator's URA.

Step 5.  Validate existing membership (no re-join):
           Invocation:
             caller          = device-profile Agent (canonical URA from credentials)
             subject         = device-profile Agent (self-attestation)
             callee          = hub-profile Agent (URA from credentials)
             ability         = federation.heartbeat
             args            = { initial: true,
                                 advertised_abilities_hash: <from local catalog> }
             causal_context  = Scalar(join_receipt_hash from credentials)
                               (this IS a membership-lineage-relevant call)
             nonce           = fresh
           Hub returns Receipt:
             { membership_status: "active",
               realm_directory_delta,
               policy_updates }
           If membership_status = "revoked": daemon halts non-essential
             ops, surfaces revocation, exits or runs in read-only mode.

Step 6.  Advertise local Agents to hub. For each hosted Agent in
         LocalAgentCatalog (hosted by this device-profile):
           Invocation:
             caller          = device-profile Agent
             subject         = the hosted Agent (self-attestation
                               permitted; device-profile owns the host)
             delegation      = (none required; caller and subject both
                                local to this daemon; daemon's signature
                                attests authority)
             callee          = hub-profile Agent
             ability         = federation.advertise_agent
             args            = { agent_uri,
                                 identity: {
                                   public_key,
                                   signing_authority: "hosted_by:<device-profile-uri>",
                                   host_attestation: <signed by device-profile>
                                 },
                                 metadata }
             causal_context  = Scalar(membership_validation_receipt)
           Then federation.advertise_abilities for the same Agent's
           AbilityDescriptors (§1.6).

Step 7.  Start heartbeat loop. Period = heartbeat_ms from membership
         response.
           Invocation:
             caller          = device-profile Agent
             subject         = device-profile Agent
             callee          = hub-profile Agent
             ability         = federation.heartbeat
             args            = { observed_load, ... }
             causal_context  = Null with reason="periodic_keepalive"
                               (per §A8: not enforced linear)

Step 8.  Start IPC server on ~/.easynet/control.sock. Daemon ready.
```

### §2.B — Hub-genesis startup

```
Step 0.  Operator has previously run `easynet pair --as-hub`:
         - profiles.hub = true in config
         - generated realm trust root keypair → realm-trust-root.pem
         - wrote ~/.easynet/realm.json with realm + trust_root_pubkey
         - did NOT write credentials.json

Step 1.  Daemon starts. Reads config. profiles.hub = true.
         No credentials.json required.

Step 2.  Embedded axon kernel initializes. Empty LocalAgentCatalog.
         Empty RealmDirectory.

Step 3.  Read ~/.easynet/local-agents.json. If first boot ever:
         - Mint URA for hub-profile Agent. Record. Persist.
         - Mint URA for device-profile Agent (this host can also be a
           normal device). Record. Persist.
         Otherwise: load existing URAs.

         Instantiate hub-profile Agent with realm trust root keypair
         (signing_authority = "self" for hub). Register in
         LocalAgentCatalog. Also add to local RealmDirectory directly
         (auto-membership for the realm authority).

         Instantiate device-profile Agent with own keypair
         (signing_authority = "self"). Add to LocalAgentCatalog and
         RealmDirectory.

         If profiles.consent / policy / mcp / etc. enabled: instantiate
         hosted Agents per §1.3 Model B. Add to both catalogs.

Step 4.  Open hub-profile Agent's federation.* gRPC endpoints (default
         :7843). Accept federation.join from joining devices.

Step 5.  Start IPC server on ~/.easynet/control.sock. Daemon ready.
```

A host MAY have profiles.hub = true AND profiles.device = true (single
combined node). The hub-profile self-admits the device-profile.

---

## §3 — `easynet join <token>`

```
$ easynet join eyJhbGciOi...

Step A.  CLI generates fresh keypair → ~/.easynet/private.pem (mode 0600).
         Computes public_key_fingerprint.

Step B.  CLI sends bootstrap invocation:
           caller              = "provisional:<public_key_fingerprint>"
           subject             = "provisional:<public_key_fingerprint>"
           delegation          = (none; provisional caller IS subject)
           callee              = hub-profile Agent (URA from token)
           ability             = federation.join
           args                = { realm, pairing_secret, public_key,
                                   host_descriptor, profiles_summary }
           causal_context      = Null (genesis exception)
           nonce               = fresh
           caller_signature    = self-sign with new private key

Step C.  Hub admission gate:
           Base layer: verify signature against embedded public_key
                       (NOT against any directory entry). Nonce unique.
                       Envelope well-formed.
           Pre-membership exception: ability == federation.join → bypass
                       membership and policy layers. Validate
                       pairing_secret. If invalid → Receipt {Failed,
                       reason: "invalid_token"}.

Step D.  Hub federation.join handler:
           Allocate canonical_agent_uri = easynet:///r/<realm>/agent/<ulid>.
           If public_key already bound to a different URI: Receipt
             {Failed, reason: "public_key_already_bound", hint:
              "regenerate keypair and retry"}. Hub does NOT rotate the
             device's key.
           Bind public_key to URA in RealmDirectory.
           Persist membership.
           Sign Receipt:
             { Ok, body: { canonical_agent_uri, membership_token,
                           hub_endpoints, heartbeat_ms,
                           realm_trust_root_pubkey, join_receipt_hash } }
           Publish receipt hash to realm transparency log.

Step E.  CLI receives Receipt.
         Writes ~/.easynet/credentials.json with canonical_agent_uri.
         Writes ~/.easynet/local-agents.json with this URA as
         host_device_agent_uri.

         join_receipt_hash is THIS DEVICE'S MEMBERSHIP LINEAGE ROOT [P3].
         Future invocations SHOULD reference it directly or transitively
         when they are membership-dependent (advertise, heartbeat,
         operator-issued commands acting under this membership). It is
         NOT a globally-required causal_context for every invocation —
         operator-initiated commands and periodic activity may use Null
         with reason per §A8.

Step F.  CLI prints: "Joined realm <realm> as <canonical_agent_uri>."
```

---

## §4 — `easynet pair --as-hub`

```
$ easynet pair --as-hub --realm acme

Step A.  CLI writes ~/.easynet/config.toml profiles.hub = true.
Step B.  CLI generates realm trust root keypair → realm-trust-root.pem.
         Writes ~/.easynet/realm.json with realm + trust_root_pubkey.
Step C.  CLI prints first device pairing token.
Step D.  Operator runs `easynet runtime start`. Daemon boots in §2.B.
```

---

## §5 — `easynet agent add claude`

```
$ easynet agent add claude --model sonnet --type claude-code

Step A.  CLI sends invocation to local daemon over ~/.easynet/control.sock:
           caller          = device-profile Agent
           subject         = operator's URA
           delegation      = local socket auth materialized as a
                             DelegationProof with issuer = device-profile
                             (the daemon attests "this connection came
                              from the local operator via 0600 socket"),
                             subject = operator URA, caller = device,
                             audience = device-profile self, scopes =
                             ["fleet.*"], short expiry
           callee          = device-profile Agent (this host)
           ability         = fleet.start_agent
           args            = { name: "claude", agent_type: "claude-code",
                               model: "sonnet" }
           causal_context  = Null with reason="operator_command"
           nonce           = fresh

Step B.  device-profile fleet.start_agent handler:
           1. Mint canonical URA for new llm-profile Agent.
              Record in ~/.easynet/local-agents.json.
           2. Create workspace ~/.easynet/workspaces/claude/.
           3. Scan ~/.claude/skills, build PRIVATE AbilityDescriptors
              with owner = new llm Agent URA, scope_subjects =
              [operator URA].
           4. Spawn handler. Register in LocalAgentCatalog with
              signing_authority = "hosted_by:device-profile-URA".
           5. federation.advertise_agent for the new llm Agent
              (caller = device-profile, subject = new llm Agent).
           6. federation.advertise_abilities for its abilities.

Step C.  Receipt to CLI: { canonical_agent_uri, advertised_abilities_count }.
Step D.  CLI prints: "Added agent claude (<URA>) with N abilities."
```

---

## §6 — `easynet skill list`

```
$ easynet skill list

Step A.  CLI sends:
           caller          = device-profile Agent
           subject         = operator's URA
           delegation      = local-socket DelegationProof (as in §5)
           callee          = hub-profile Agent
           ability         = federation.resolve
           args            = { filter: { has_ability_prefix: "skill." } }
           causal_context  = Null with reason="operator_command"

Step B.  Hub admission: base + membership (caller is realm member) +
         policy. Allowed.

Step C.  Hub federation.resolve handler:
           Scan RealmDirectory for Agents with abilities matching prefix.
           Apply visibility filter per §1.6:
             PRIVATE skills returned only if (subject is host operator
             OR caller is the host device of the skill's owner).
           Returns Receipt: { agents: [{ uri, abilities: [Ability
           Descriptor, ...] }, ...] }.

Step D.  CLI receives, renders grouped by owner Agent.
```

---

## §7 — `easynet ability invoke claude.skill.alive-video`

```
Step A.  CLI parses callee URA + ability.
Step B.  CLI sends:
           caller          = device-profile Agent
           subject         = operator
           delegation      = local-socket DelegationProof
           callee          = <local claude llm-profile Agent URA>
           ability         = skill.alive-video
           args            = ...
           causal_context  = Null with reason="operator_command"
                             OR Scalar(prev session receipt)

Step C.  Daemon admission gate:
           Base ok.
           Membership: caller in realm.
           Delegation: subject=operator, caller=device-profile, scopes
             include "skill.*" (or the specific name), audience is
             local. Pass.
           Policy: invoke policy-profile (in-process). Sub-invocation:
             caller              = device-profile  (the hosting Agent)
             subject             = original subject (operator)
             callee              = policy-profile
             ability             = policy.evaluate
             args                = { invocation_envelope }
             admission_internal  = true   [P4: kernel-local header,
                                            never accepted from remote]
             causal_context      = Scalar(<original envelope hash>)
           policy.evaluate returns Allowed.

Step D.  Dispatch via LocalAgentCatalog → claude Agent's
         skill.alive-video handler (in-process).

Step E.  Handler returns. Daemon constructs Receipt:
           callee_agent_uri  = claude Agent URA
           signer_agent_uri  = device-profile Agent URA  [P1, hosted Model B]
           host_attestation  = signed by device-profile
           body              = handler result
           causal_context    = (carried from envelope)
         Daemon signs Receipt with device-profile key. Persists.

Step F.  CLI prints result.
```

---

## §8 — `easynet ability invoke claude.conversation.send`

Same shape as §7, ability = `conversation.send`. For streaming variant,
daemon uses InvokeStream.

Note: per [P8], `conversation.send` and `conversation.stream` default
visibility is **SCOPED**, with scope_subjects = [host operator's URA].
A remote caller without the operator in scope cannot invoke them
unless `meta.publish` has changed visibility to PUBLIC.

---

## §9 — Frontend Skills page

```
[Browser] (application layer)
  GET /api/v1/skills/installed   with JWT Bearer

[Backend] (NODE)
  Validates JWT → operator URA.
  Constructs DelegationProof:
    issuer_uri  = backend-profile Agent URA
    subject_uri = operator URA
    caller_uri  = backend-profile Agent URA
    audience    = hub-profile URA
    scopes      = ["federation.resolve", "meta.list_abilities"]
    issued_at, expires_at = JWT validity window
    signature   = backend's key

  Issues:
    caller          = backend-profile Agent
    subject         = operator URA
    delegation      = the DelegationProof above
    callee          = hub-profile Agent
    ability         = federation.resolve
    args            = { filter: { has_ability_prefix: "skill." } }
    causal_context  = Scalar(prev session receipt) OR Null
                      with reason="ui_session_query"
    nonce           = fresh
    caller_signature= backend's key

  → axon SDK → gRPC Invoke → axon kernel on hub host.

[Hub host's daemon]
  Admission:
    Base: backend's signature verifies.
    Membership: backend-profile in directory.
    Delegation: per §1.5 — issuer=backend trusted, subject signature
      chain ok, caller=backend matches envelope, audience matches hub,
      scopes include federation.resolve. Pass.
    Policy: visibility filter per §1.6.
  hub.federation.resolve runs.
  Returns Receipt { agents: [...filtered...] } signed by hub.

[Backend]
  Optionally fans out per-Agent meta.list_abilities for richer detail
  (same delegation pattern). Reshapes into existing { items:
  InstalledSkill[] } HTTP response.

[Browser]
  Renders. Each row carries owner_agent_uri.
```

No node-to-node MCP. Backend signs as backend-profile. Operator's
authority flows via subject + DelegationProof.

---

## §10 — Frontend Abilities page

Same as §9 with empty filter. Returns federation-wide ability catalog.

---

## §11 — Frontend chat page

```
[Browser]
  POST /api/v1/abilities/invoke
    body = { callee_agent_uri, ability: "conversation.send",
             args: { prompt } }

[Backend]
  Validates JWT → operator URA.
  DelegationProof: same shape as §9 but with audience = callee daemon,
    scopes = ["conversation.send"].
  Invocation:
    caller         = backend-profile Agent
    subject        = operator URA
    delegation     = DelegationProof
    callee         = <from request>
    ability        = conversation.send
    args           = { prompt }
    causal_context = Scalar(prev http session receipt) OR Null
  → gRPC to remote daemon (or in-process if backend co-resides).

[Daemon hosting LLM Agent]
  Admission. Note: `conversation.send` is SCOPED [P8]. Visibility check:
  scope_subjects must include operator URA. If operator owns the host of
  this LLM, pass. Else policy must explicitly allow (e.g. via
  `meta.publish` to make it PUBLIC, or operator added to scope_subjects).
  Dispatch. Streaming via InvokeStream chunks.

[Backend]
  Forwards chunks to Frontend.
```

---

## §12 — External Claude Code via MCP

```
[Claude Code app]
  mcp.servers.easynet = { url: "ipc:///Users/.../easynet/mcp.sock" }
  Issues MCP tools/list.

[Daemon's mcp-profile Agent] [P6: single Agent with both bridge + client abilities]
  Receives MCP tools/list on socket.
  Authenticates connection (filesystem permissions identify operator).

  Translates to in-process Invocation:
    caller          = mcp-profile Agent
    subject         = local operator
    delegation      = local-socket DelegationProof
    callee          = hub-profile Agent (in-process if local hub, else gRPC)
    ability         = federation.resolve
    args            = { filter: {
                          visibility: ["PUBLIC"],
                          scope_includes: subject_uri    // includes SCOPED matching this operator
                       } }
                      // PRIVATE NOT included by default

  Receipt with abilities filtered to PUBLIC + SCOPED-matching.

  ADDITIONAL local projection (mcp-profile config option,
  project_local_private_for_owner, default true):
    For PRIVATE abilities owned by Agents whose host is THIS daemon
    AND owner's operator scope matches the connecting operator,
    mcp-profile may project them as ephemeral SCOPED entries IN THIS
    MCP SESSION ONLY. They are NOT advertised to the realm; they exist
    only within this MCP server response.

  Synthesizes MCP tool entries. Returns to Claude Code.

[Claude Code]
  Calls tools/call:
    {"name": "claude.skill.design", "arguments": {...}}

[Daemon's mcp-profile Agent]
  Translates:
    caller         = mcp-profile Agent
    subject        = local operator
    delegation     = local-socket DelegationProof
    callee         = <claude Agent URA>
    ability        = skill.design
    args           = <translated>
    causal_context = Null with reason="mcp_bridge_session"
  Dispatch via LocalAgentCatalog (in-process).
  Receipt back. Translates to MCP tools/call response.

[Claude Code]
  Continues.
```

PRIVATE abilities never land in remote realm queries. They surface to
MCP only via `project_local_private_for_owner` on local mcp-profile.

---

## §13 — Outbound MCP

```
[Some EasyNet handler decides to call external MCP]
  Issues:
    caller         = <calling Agent>
    subject        = relevant subject
    callee         = local mcp-profile Agent     [P6: same Agent as §12]
    ability        = mcp.client.call
    args           = { server_uri, tool_name, args }

[Daemon's mcp-profile Agent]
  Looks up / opens MCP client connection.
  Translates Invocation → MCP tools/call. Sends.
  Receives MCP response. Wraps in Receipt. Returns.
```

The same `mcp-profile` Agent provides both inbound (`mcp.bridge.*`) and
outbound (`mcp.client.*`) abilities.

---

## §14 — Permission / consent

```
[Original Invocation enters daemon]
  e.g. ability = fleet.stop_agent on production-claude.

[Daemon kernel admission gate]
  Base, Membership, Delegation: ok.
  Policy layer:
    Build sub-invocation:
      caller              = device-profile Agent (hosting Agent of this kernel)
      subject             = original subject
      callee              = policy-profile Agent
      ability             = policy.evaluate
      args                = { invocation_envelope: <original> }
      admission_internal  = true   [P4: kernel-local execution flag]
      causal_context      = Scalar(<original envelope hash>)

    [P4 enforcement] Before dispatching this sub-invocation, the
    kernel verifies the sub-invocation was constructed locally (not
    received from network). Any inbound network envelope carrying
    admission_internal=true is REJECTED at the transport layer
    BEFORE reaching admission. The flag is never serializable from
    untrusted sources.

    Dispatch in-process. Receipt: {Pending, consent_request: {...}}.

  Build:
    caller              = device-profile Agent
    subject             = original subject
    callee              = consent-profile Agent
    ability             = consent.request
    args                = { original_invocation, risk_class, ui_hint, timeout }
    admission_internal  = true
    causal_context      = Scalar(policy_evaluate_receipt)

  consent-profile stores in pending queue, notifies subscribers.

[Frontend (subscribed via consent.subscribe InvokeStream)]
  Receives pending request. Modal pops up. User clicks Allow.
  Frontend issues:
    caller         = backend-profile Agent
    subject        = operator URA
    delegation     = JWT-derived DelegationProof
    callee         = consent-profile Agent
    ability        = consent.decide
    args           = { request_id, decision: "Allowed", justification }

  consent-profile records decision. Returns Receipt to original
  consent.request caller (admission gate still waiting).

[Admission gate]
  Sees Allowed. Continues original Invocation dispatch.

[fleet.stop_agent handler]
  Stops agent. Returns Receipt.
```

`admission_internal=true` is a kernel-local execution flag. Cannot be
set by remote callers. (Caller is always a real Agent — device-profile
in this example.)

---

## §15 — Heartbeat, membership refresh, revocation

```
caller         = device-profile Agent
subject        = device-profile Agent
delegation     = (none; subject == caller)
callee         = hub-profile Agent
ability        = federation.heartbeat
args           = { observed_load, advertised_abilities_hash }
causal_context = Null with reason="periodic_keepalive"
                 OR Scalar(prev_heartbeat) at daemon's discretion
```

Hub returns `{ realm_directory_delta, policy_updates, status }`.
Revocation handling: daemon updates LocalAgentCatalog; in-flight
Invocations for revoked Agents terminate with {Failed, REVOKED}.

---

## §16 — Cross-device invocation

```
$ easynet ability invoke easynet:///r/acme/agent/01LLM-host-B-claude.skill.design

[CLI on host-A]
  Sends to host-A's daemon via IPC:
    caller         = device-profile Agent (host-A)
    subject        = operator
    delegation     = local-socket DelegationProof
    callee         = <host-B claude URA>
    ability        = skill.design
    args           = ...
    causal_context = Null with reason="operator_command"

[host-A daemon]
  Admission ok.
  Dispatch:
    1. LocalAgentCatalog lookup → MISS.
    2. Cached RealmDirectory → HIT (host-B endpoints known).
    3. Build outbound gRPC Invoke. Sign with host-A device-profile key.
       Forward to host-B endpoint.
       The original caller (host-A device-profile) and subject
       (operator) are carried through. host-A is the signing /
       transmitting node; host-A's signature attests to the relay.

[host-B daemon]
  Receives gRPC Invoke. Admission:
    Base: host-A's signature verifies (host-A device-profile in
      host-B's RealmDirectory).
    Membership: host-A is realm member.
    Delegation: per §1.5 — DelegationProof's caller=host-A device,
      subject=operator, scopes include "skill.*". Pass.
    Policy: visibility check — skill.design is PRIVATE on host-B's
      claude Agent, scope_subjects = [host-B operator]. Operator URA
      from host-A is the SAME operator (assuming single tenancy) →
      pass. Otherwise → DENY.
  Dispatch via host-B LocalAgentCatalog.
  Receipt back to host-A.

[host-A daemon]
  Forwards Receipt to CLI.

[CLI]
  Prints result.
```

**Hub-down resilience clause (§A11):** after a successful prior
`federation.resolve` for the target and while the cached RealmDirectory
entry is unexpired, cross-device invocation MUST proceed with hub
stopped. Without cached entry → cross-device discovery requires hub.

---

## §17 — Old-client cross-version negative

```
[Old Go SDK]
  Calls axon.Client.RegisterNode(...)
  → gRPC method service.Axon/RegisterNode
  → Method not found.
  → Returns gRPC UNIMPLEMENTED with default message.

[Old client]
  Receives UNIMPLEMENTED. Logs. Surfaces to operator.
```

Old RPCs MUST return UNIMPLEMENTED, unknown method, or parse error.
MUST NOT silently succeed. MUST NOT route through compatibility shim.
Custom error message text is best-effort, not required. (§A9.)

---

## §18 — Standard ability registry

(Full table; covers every ability referenced in this document.
Visibility column reflects [P8] correction for conversation.*.)

| Namespace | Ability | Owner profile | Default Visibility | Input | Receipt body |
|---|---|---|---|---|---|
| federation | join | hub | PUBLIC (pre-membership exception) | `{realm, pairing_secret, public_key, host_descriptor, profiles_summary}` | `{canonical_agent_uri, membership_token, hub_endpoints[], heartbeat_ms, realm_trust_root_pubkey, join_receipt_hash}` |
| federation | leave | hub | SCOPED to caller agent | `{reason}` | `{ack}` |
| federation | resolve | hub | PUBLIC callable, policy-filtered result | `{filter: {has_ability_prefix?, visibility?, scope_includes?, agent_uri?}}` | `{agents: [{uri, identity, abilities[], endpoints[], metadata, status}]}` |
| federation | advertise_agent | hub | SCOPED to caller agent | `{agent_uri, identity, metadata}` | `{ack, replaced_prior}` |
| federation | advertise_abilities | hub | SCOPED to caller agent | `{agent_uri, abilities[]}` | `{ack, replaced_prior}` |
| federation | heartbeat | hub | SCOPED to caller agent | `{observed_load, advertised_abilities_hash, initial?}` | `{membership_status, realm_directory_delta, policy_updates}` |
| federation | revoke | hub | SCOPED (admin) | `{agent_uri, reason}` | `{ack}` |
| federation | federate | hub | SCOPED (admin) | `{other_hub_uri, trust_root}` | `{federation_link_id}` |
| transport | relay.invoke | hub (opt-in) | SCOPED | `{target_invoke_envelope}` | passthrough Receipt |
| transport | relay.stream | hub (opt-in) | SCOPED | same | streamed Receipt chunks |
| identity | rotate_key | hub | SCOPED (subject auth required) | `{agent_uri, new_pubkey, attestation}` | `{rotated_at, prior_key_id}` |
| identity | revoke_key | hub | SCOPED | `{key_id, reason}` | `{ack}` |
| identity | issue_token | hub | SCOPED | `{purpose, ttl}` | `{token}` |
| fleet | list_agents | device | SCOPED to local operator | `{}` | `{agents[]}` |
| fleet | list_abilities | device | SCOPED | `{visibility_filter?, name_prefix?}` | `{abilities[]}` |
| fleet | list_sessions | device | SCOPED | `{agent_uri?}` | `{sessions[]}` |
| fleet | start_agent | device | SCOPED | `{name, agent_type, model?}` | `{canonical_agent_uri}` |
| fleet | stop_agent | device | SCOPED | `{name_or_uri}` | `{ack}` |
| fleet | attach_session | device | SCOPED | `{session_id}` | streaming chunks |
| fleet | session_input | device | SCOPED | `{session_id, data}` | `{ack}` |
| fleet | session_read | device | SCOPED | `{session_id}` | streaming chunks |
| fleet | session_resize | device | SCOPED | `{session_id, cols, rows}` | `{ack}` |
| fleet | session_close | device | SCOPED | `{session_id}` | `{ack}` |
| admin | failover | device | SCOPED (admin) | `{target}` | `{outcome, new_primary}` |
| admin | snapshot | device | SCOPED (admin) | `{}` | `{snapshot_id}` |
| admin | recover | device | SCOPED (admin) | `{snapshot_id}` | `{outcome}` |
| admin | status | device | SCOPED | `{}` | `{status, components[]}` |
| observe | health | device | PUBLIC | `{}` | `{status, details}` |
| observe | network_health | device | PUBLIC | `{}` | `{links[], latency, ...}` |
| meta | describe | any | PUBLIC | `{}` | `{uri, identity_summary, abilities_summary, metadata}` |
| meta | list_abilities | any | callable PUBLIC, results filtered per AbilityDescriptor visibility | `{visibility_filter?, name_prefix?}` | `{abilities: AbilityDescriptor[]}` |
| meta | acquire | any | SCOPED to owner | `{ability_source}` | `{acquired_ability_name}` |
| meta | forget | any | SCOPED to owner | `{ability_name}` | `{ack}` |
| meta | publish | any | SCOPED to owner | `{ability_name, new_visibility, scope?}` | `{ack}` |
| meta | compose | any | SCOPED to owner | `{component_abilities[], new_name}` | `{composed_ability_name}` |
| meta | cancel | any | SCOPED | `{invocation_id}` | `{ack, prior_state}` |
| consent | request | consent | SCOPED to admission callers | `{original_invocation, risk_class, ui_hint, timeout}` | `{decision: Allowed\|Denied\|Timeout, request_id}` |
| consent | subscribe | consent | SCOPED to UI | `{}` | streaming pending requests |
| consent | decide | consent | SCOPED to UI | `{request_id, decision, justification}` | `{ack}` |
| consent | list_pending | consent | SCOPED to UI | `{}` | `{requests[]}` |
| policy | evaluate | policy | SCOPED to admission callers | `{invocation_envelope}` | `{decision, reason, expires_at}` |
| policy | simulate | policy | SCOPED | `{invocation_envelope}` | `{would_decide, trace}` |
| policy | publish | policy | SCOPED (admin) | `{policy_doc}` | `{policy_id}` |
| policy | list | policy | SCOPED (admin) | `{}` | `{policies[]}` |
| schedule | add | device | SCOPED | `{cron_expr, target_invocation_template}` | `{schedule_id}` |
| schedule | list | device | SCOPED | `{}` | `{schedules[]}` |
| schedule | remove | device | SCOPED | `{schedule_id}` | `{ack}` |
| schedule | enable | device | SCOPED | `{schedule_id, enabled}` | `{ack}` |
| loop | create | device | SCOPED | `{spec}` | `{loop_id}` |
| loop | status | device | SCOPED | `{loop_id}` | `{state}` |
| loop | subscribe | device | SCOPED | `{loop_id}` | streaming events |
| loop | cancel | device | SCOPED | `{loop_id}` | `{ack}` |
| conversation | send | llm | **SCOPED [P8]** to host operator by default | `{prompt, options}` | `{response}` |
| conversation | stream | llm | **SCOPED [P8]** to host operator by default | `{prompt, options}` | streaming chunks |
| session | create | llm | SCOPED | `{config}` | `{session_id}` |
| session | list | llm | SCOPED | `{}` | `{sessions[]}` |
| session | resume | llm | SCOPED | `{session_id}` | `{ack}` |
| session | close | llm | SCOPED | `{session_id}` | `{ack}` |
| skill | (per-skill names) | llm | PRIVATE | per-skill | per-skill |
| mcp.bridge | list_tools | mcp [P6] | SCOPED to local MCP clients | `{}` | MCP-shaped |
| mcp.bridge | call_tool | mcp [P6] | SCOPED to local MCP clients | `{tool_name, args}` | MCP-shaped |
| mcp.client | list | mcp [P6] | SCOPED | `{}` | `{servers[]}` |
| mcp.client | call | mcp [P6] | SCOPED | `{server_uri, tool_name, args}` | `{result}` |
| aggregate | list_skills_across_fleet | backend | SCOPED to operator | `{tenant_filter?}` | `{skills_by_device[]}` |
| aggregate | list_abilities_catalog | backend | SCOPED to operator | `{}` | `{ability_catalog[]}` |

---

## §19 — Function-to-mechanism mapping (inlined; was omitted in v4.1) [P7]

For each current EasyNet function, the v4.1.1 invocation chain that
implements it:

| Function | Caller | Subject | Delegation | Callee | Ability | Section |
|---|---|---|---|---|---|---|
| `easynet runtime start` (joined-device) | device-profile | device-profile | none | hub-profile | `federation.heartbeat` (initial=true) | §2.A Step 5 |
| `easynet runtime start` (joined-device) | device-profile | each hosted Agent | none (self-attest) | hub-profile | `federation.advertise_agent` + `federation.advertise_abilities` | §2.A Step 6 |
| `easynet runtime start` (hub-genesis) | (no network call; instantiate locally) | — | — | — | — | §2.B |
| `easynet runtime stop` | device-profile | device-profile | none | hub-profile | `federation.leave` | (analogous to §15) |
| `easynet join <token>` | provisional:fingerprint | provisional:fingerprint | none | hub-profile | `federation.join` | §3 |
| `easynet pair --as-hub` | (no network call; writes config + keys) | — | — | — | — | §4 |
| `easynet agent add <name>` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `fleet.start_agent` | §5 |
| `easynet agent remove <name>` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `fleet.stop_agent` | (analogous to §5) |
| `easynet agent list` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `fleet.list_agents` OR `meta.describe` | (analogous to §6) |
| `easynet skill list` | device-profile | operator | local-socket DelegationProof | hub-profile | `federation.resolve` (filter prefix `skill.`) | §6 |
| `easynet skill install <source>` | device-profile | operator | local-socket DelegationProof | target llm-profile Agent | `meta.acquire` | (analogous to §5) |
| `easynet skill remove <name>` | device-profile | operator | local-socket DelegationProof | target llm-profile Agent | `meta.forget` | (analogous to §5) |
| `easynet ability list` | device-profile | operator | local-socket DelegationProof | hub-profile | `federation.resolve` (no filter) | §10 |
| `easynet ability invoke <name>` | device-profile | operator | local-socket DelegationProof | target Agent (parsed from name) | per ability | §7, §8 |
| `easynet ability cancel <id>` | device-profile | operator | local-socket DelegationProof | target Agent | `meta.cancel` | (analogous to §7) |
| `easynet session list` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `fleet.list_sessions` | (analogous to §6) |
| `easynet session attach <id>` | device-profile | operator | local-socket DelegationProof | device-profile (self) | InvokeStream `fleet.attach_session` | §16-style streaming |
| `easynet schedule add` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `schedule.add` | (analogous) |
| `easynet loop create` | device-profile | operator | local-socket DelegationProof | device-profile (self) | `loop.create` | (analogous) |
| `easynet permission ...` | device-profile | operator | local-socket DelegationProof | consent-profile | `consent.list_pending` / `consent.decide` | §14 |
| Frontend Skills page | backend-profile | operator | JWT-derived DelegationProof | hub-profile | `federation.resolve` (filter `skill.`) | §9 |
| Frontend Abilities page | backend-profile | operator | JWT-derived DelegationProof | hub-profile | `federation.resolve` (no filter) | §10 |
| Frontend Devices page | backend-profile | operator | JWT-derived DelegationProof | hub-profile | `federation.resolve` (filter `fleet.*`) | (analogous to §10) |
| Frontend Agents page | backend-profile | operator | JWT-derived DelegationProof | hub-profile | `federation.resolve` (no filter) | (analogous to §10) |
| Frontend chat | backend-profile | operator | JWT-derived DelegationProof | target llm-profile Agent | `conversation.send` (or stream) | §11 |
| Frontend session attach | backend-profile | operator | JWT-derived DelegationProof | target device-profile | InvokeStream `fleet.session_read` + `fleet.session_input` | (analogous to §16) |
| Frontend permission popup | backend-profile | operator | JWT-derived DelegationProof | consent-profile | InvokeStream `consent.subscribe` + `consent.decide` | §14 |
| External Claude Code via MCP (tools/list) | mcp-profile | local operator | local-socket DelegationProof | hub-profile | `federation.resolve` (filter PUBLIC + SCOPED-matching) | §12 |
| External Claude Code via MCP (tools/call) | mcp-profile | local operator | local-socket DelegationProof | target Agent | per tool name | §12 |
| Daemon outbound to external MCP | <calling Agent> | relevant subject | (none if calling Agent is local) | mcp-profile (self) | `mcp.client.call` | §13 |
| Cross-device invocation | device-profile (host-A) | operator | local-socket DelegationProof | <remote Agent on host-B> | per ability | §16 |
| Heartbeat (periodic) | device-profile | device-profile | none | hub-profile | `federation.heartbeat` | §15 |
| Old SDK cross-version | (any) | (any) | — | (deleted RPC) | — (UNIMPLEMENTED) | §17 |

This table, combined with §18, is the binding contract.

---

## §20 — What can break and how it's caught

| Risk | Detection |
|---|---|
| Function in §19 returns wrong shape | E2E smoke per row |
| Causal context chain broken | Receipt chain smoke verifies inbound context embedded in outbound receipt |
| Deleted RPC has no ability replacement | P1 restatement mapping doc exhaustive; CI fails on missing entry |
| MCP leaks into node-to-node path | Conformance grep: disallows `/\bmcp\b/i` outside `agents/mcp.rs` (the single mcp-profile module) and outside backend non-existence |
| Agent gets a `kind` field added | Conformance grep: disallows enum AgentKind / AgentRole in any proto |
| Permission flow falls back to old broker | Conformance grep: disallows `system.permission.*` |
| Hub becomes data-path required | E2E test with prior cache + hub down |
| Caller spoofing (Frontend writing `caller=human-agent`) | Backend SDK signs envelopes with backend key automatically; subject is set from JWT; SDK does not allow caller override |
| Subject delegation forged | DelegationProof signature verified at admission gate per §1.5 |
| PRIVATE skill leaks via MCP | Conformance test: MCP tools/list with fake remote operator; PRIVATE never appears unless project_local_private_for_owner AND local connection |
| Hosted Agent receipt signing confusion | Receipt schema must include `signer_agent_uri` distinct from `callee_agent_uri` for hosted Agents; verified by smoke test |
| Hosted Agent URI churn across restarts | Smoke test: stop daemon → start daemon → `easynet ability list` shows same URAs |
| `admission_internal=true` from network | Transport layer rejects inbound envelopes with this flag set; verified by negative test |
| Old client silent success | Cross-version negative test |

---

## §A — Ontology Consistency Checklist (binding constraints)

Future revisions MUST satisfy every item below.

| # | Rule | Verifier |
|---|---|---|
| A1 | Invoke envelope distinguishes caller (signer/transmitter) from subject (principal). Backend never claims caller=human-agent. | §A4, §9, §10, §11 |
| A2 | Provisional URA pre-membership; canonical URA assigned in federation.join receipt; never reused. Hub never rotates a device's key on collision. | §3 |
| A3 | Daemon has two startup modes (joined-device requires credentials.json; hub-genesis does not). Not in conflict. | §2.A, §2.B |
| A4 | The word "role" never appears in proto, admission/dispatch code, or user config. Use "profile" for ability bundles. enum AgentRole / Agent.kind forbidden. | §1, CI grep |
| A5 | PRIVATE Abilities never auto-surface in remote queries. mcp-profile MAY explicitly project local PRIVATE Abilities into ephemeral SCOPED MCP entries for the connecting local operator only, configured via `project_local_private_for_owner`. | §6, §12 |
| A6 | Admission gate is a kernel mechanism, NOT a caller. Sub-invocations carry envelope header `admission_internal=true`. Caller is the hosting Agent. The header bypasses recursive policy admission only and CANNOT be set by external callers — kernel-local only. [P4] | §7, §14, §20 (negative test) |
| A7 | Two distinct catalogs: LocalAgentCatalog (per-daemon, dispatch handles) and RealmDirectory (per-realm, discovery facts). | §1.2 |
| A8 | causal_context ∈ {Null, Scalar, Vector, DAG}. Genesis (federation.join, operator-initiated commands) MAY be Null with explicit reason. Periodic activity (heartbeats) MAY be Null with reason. **join_receipt_hash is the membership lineage root, not a globally-required causal_context for every invocation.** [P3] | §2.A Step 7, §3 Step E, §15 |
| A9 | Old RPC negative behavior requires UNIMPLEMENTED / unknown method / parse error. Custom migration message text best-effort. | §17 |
| A10 | Every ability referenced anywhere in this document appears in the §18 Standard Ability Registry. | §18 |
| A11 | "Hub down → cross-device invocation works" requires prior cache warmup. Without cached RealmDirectory entry for the target, hub is required for resolve. | §16 |
| A12 | **Hosted child Agents (Model B) sign receipts via the hosting device-profile, with `signer_agent_uri` distinct from `callee_agent_uri` and a `host_attestation` field. Self-signing Agents (Model A) sign their own receipts. The choice is recorded in DirectoryEntry.identity.signing_authority.** [P1] | §1.3, §7 Step E |
| A13 | **Hosted Agent URIs are minted by the hosting device-profile, persisted to ~/.easynet/local-agents.json, and reused across daemon restarts. Hub accepts them only via `federation.advertise_agent` from the hosting device-profile.** [P2] | §1.4 |
| A14 | **DelegationProof schema (issuer, subject, caller, audience, scopes, issued_at, expires_at, signature). Verification at admission gate per §1.5 rules. JWT verification is not hardcoded against operator public key — it goes through the DelegationProof model.** [P5] | §1.5, §9, §11 |
| A15 | **AbilityDescriptor includes owner_agent_uri, visibility, scope_subjects[], scope_agents[], source, schema_summary, hints. PRIVATE is a degenerate SCOPED case: scope = {host operator}.** [P9] | §1.6, §6, §18 |

---

## §21 — Approval gate

After your approval of v4.1.1, implementation may start. The
implementation sequence is unchanged from v3:

```
P0  Conformance lock           — CI guards, no functional change
P1  Axon protocol cut          — Invoke + InvokeStream only; old RPCs deleted
P2  Agent catalog + dispatch   — single agents table; node-to-node Invocation routing
P3  Federation bootstrap       — federation.join + advertise + resolve;
                                  three-layer admission with admission_internal=true
                                  enforcement at transport layer
P4  CLI agent profiles         — device, hub, consent, policy, mcp-profile,
                                  llm-profile; local-agents.json persistence;
                                  hosted Agent receipt signing model;
                                  system.* deleted
P5  Backend node-to-node       — backend.axon.Client collapses to
                                  Invoke + InvokeStream + Close; DelegationProof
                                  construction for JWT-bearing requests;
                                  listInstalledLogic + pty_driver rewritten
P6  Frontend & negative        — Frontend stays HTTP; verify end-to-end Skills,
                                  conformance grep zero, old-client cross-version
                                  negative test, admission_internal=true negative test
```

The conformance scripts from P0 enforce every §A constraint.

If you confirm v4.1.1 as the binding contract, please reply with
"approved" or specify which section needs further revision (will become
v4.1.2). After approval, I begin P0.
