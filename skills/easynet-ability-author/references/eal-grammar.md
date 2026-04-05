# EAL Grammar Reference

## Tokens

```
KEYWORDS: mission, call, on, with, let, timeout, retries, on_failure, optional
FAILURE_POLICIES: abort, skip, retry, continue
LITERALS: "string", 42, 3.14, true, false
VARREF: identifier.output (e.g., photo.output)
COMMENT: // single line
```

## Grammar (EBNF)

```
program     = mission_decl
mission_decl = "mission" STRING "{" statement* "}"

statement   = let_call | bare_call
let_call    = "let" IDENT "=" call_expr
bare_call   = call_expr

call_expr   = "call" STRING ("on" STRING)? ("with" "{" field_list "}")? options*
field_list  = field ("," field)* ","?
field       = IDENT "=" field_value
field_value = STRING | INT | FLOAT | BOOL | var_ref
var_ref     = IDENT "." "output"

options     = "timeout" INT
            | "retries" INT
            | "on_failure" FAILURE_POLICY
            | "optional"
```

## Compilation Pipeline

```
source → lexer → parser → analyzer → planner → ir → interpreter
                              │           │
                              │           └── Phase partitioning (topological sort)
                              └── Symbol resolution + cycle detection
```

## Phase Rules

- Steps with NO dependencies → Phase 0 (all parallel)
- Steps depending on Phase N → Phase N+1
- Steps within same phase have NO mutual dependencies → safe to parallelize

## Data Flow

- `let x = call ...` captures the step's JSON result
- `input = x.output` references the captured result in a later step
- This creates an edge in the dependency DAG
- The compiler detects cycles (A→B→A) and rejects them

## Agent Dispatch Convention

When `target_node_id` matches a registered agent name:

1. `function_name` → task description
2. `arguments.prompt` → sent as the agent's input (if present)
3. Other arguments → included as `key: value` context lines
4. Response → `{"ok": true, "agent": "...", "output": "...", "duration_ms": N}`
5. Downstream steps reference `var.output` → gets the `output` field
