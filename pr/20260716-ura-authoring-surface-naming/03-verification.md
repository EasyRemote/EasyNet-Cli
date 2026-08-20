# Verification

Executed checks:

```sh
rg -n "axon-resource-uri|project_uri|\\bURI\\b|\\buri\\b" \
  docs/AGENT_IDENTITY.md \
  examples/claude-skill/skill/SKILL.md \
  skills/easynet-ability-author/SKILL.md \
  skills/easynet-pages-author/SKILL.md
```

Result: no matches. `rg` returned `1`, which is the expected status for an
empty match set.

```sh
bash tools/scripts/check-sdk-ura-naming.sh --self-test
bash tools/scripts/check-sdk-ura-naming.sh
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
```

Result: all passed.

`git diff --cached --check` is run after staging.
