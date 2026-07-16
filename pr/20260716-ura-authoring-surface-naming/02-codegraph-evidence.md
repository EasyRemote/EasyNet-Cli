# CodeGraph-style evidence

Commands used before staging:

```sh
rg -n "axon-resource-uri|project_uri|\\bURI\\b|\\buri\\b" \
  docs/AGENT_IDENTITY.md \
  examples/claude-skill/skill/SKILL.md \
  skills/easynet-ability-author/SKILL.md \
  skills/easynet-pages-author/SKILL.md
```

Result: no matches after the in-scope cleanup.

```sh
rg -n "axon-resource-ura|project_ura|\\bURA\\b" \
  docs/AGENT_IDENTITY.md \
  examples/claude-skill/skill/SKILL.md \
  skills/easynet-ability-author/SKILL.md \
  skills/easynet-pages-author/SKILL.md
```

Result: the live authoring fields now use `axon-resource-ura` and
`project_ura`, and the identity guide uses URA terminology for the L3 boundary.

```sh
rg -n "axon-resource-uri|project_uri|\\bURI\\b" \
  docs examples skills --glob '!docs/rfc/**' --glob '!docs/reviews/**' \
  --glob '!**/*.bak'
```

Result: remaining hits are outside this live authoring slice, mostly historical
planning/RFC text or unrelated docs requiring separate ownership decisions.
