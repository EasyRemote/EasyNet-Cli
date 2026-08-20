API contract
============

`schedule.add`
--------------

Required fields:

- `target_node`
- `target_agent`
- `cron_expr`
- `misfire_policy`
- `prompt`

`prompt` must be a non-empty string after trimming. It may contain the bounded
template variables supported by `render_prompt`.

Errors
------

- Missing or non-string prompt: `schedule: required field prompt (string)
  missing`.
- Blank prompt: `schedule.add: prompt must be non-empty`.
- Obsolete persisted rows with null/missing/blank prompt are rejected during
  store parsing and are not inserted into the runtime cache.
