# API Contract

## Request

`skill.publish` continues to accept:

```json
{
  "owner_agent_id": "<agent name>",
  "skill_name": "<dir-safe slug>",
  "skill_md": "<SKILL.md body>",
  "mission_run_id": "<optional curator run id>"
}
```

## Response

The public response remains:

```json
{
  "ok": true,
  "owner_agent_id": "<agent name>",
  "skill_name": "<slug>",
  "skill_dir": "<absolute path>",
  "content_hash": "sha256:<hex>",
  "mission_run_id": "<only when supplied>"
}
```

## Install record

When `mission_run_id` is supplied:

```json
{"source": {"kind": "curator", "identifier": "<mission_run_id>"}}
```

When absent:

```json
{"source": {"kind": "direct_publish", "identifier": "skill.publish"}}
```

The direct state is generic runtime provenance and does not encode product
names.
