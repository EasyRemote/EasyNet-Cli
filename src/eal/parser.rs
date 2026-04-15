// EasyNet CLI — EAL Parser
// ========================
//
// File: src/eal/parser.rs
// Description: Recursive descent parser — converts token stream to EalProgram AST.
//
// Grammar (EBNF):
//   program     = mission_decl
//   mission     = "mission" STRING "{" statement* "}"
//   statement   = "let" IDENT "=" rhs | rhs
//   rhs         = call_expr | member_call
//   call_expr   = "call" STRING ("on" STRING)? ("with" "{" field_list "}")? option*
//   member_call = IDENT "." IDENT "(" named_arg_list? ")" option*
//   named_arg_list = named_arg ("," named_arg)*
//   named_arg   = IDENT ":" arg_value
//   arg_value   = STRING | INT | FLOAT | BOOL | var_ref
//   var_ref     = IDENT "." "output"
//   option      = "timeout" INT | "retries" INT | "on_failure" POLICY | "optional"
//   field       = IDENT "=" (STRING | INT | FLOAT | BOOL | IDENT "." "output")
//
// EAL surface invariant (LOAD-BEARING — see docs/AGENT_IDENTITY.md):
//
//   member-call form (agent.ability) is the ONLY way to invoke an agent.
//   traditional call form (call ... on ...) is STRICTLY device-only.
//   No implicit agent fallback is allowed.
//
// The two surface productions intentionally lower to DIFFERENT IR
// target variants:
//
//   `claude.chat(prompt: "hi")`               → IrTarget::Agent(AgentId)
//   `call "chat" on "node-1" with {...}`      → IrTarget::Device { node_id }
//
// `CallExpr.target_kind: TargetKind` records which production matched
// at parse time. The planner reads it directly when lowering — there
// is no runtime classification, no `is_agent` string lookup, and no
// silent fallback from device to agent.
//
// If a user writes `call "chat" on "claude"` (traditional form, name
// collides with a registered agent), the system rejects it at
// run_mission_inproc time with an error pointing at the correct
// member-call form. See `cli/mission_runs.rs::find_implicit_agent_fallback`
// and the `no_implicit_agent_fallback_*` test trio in the same file.
//
// Grammar ambiguity guard:
//   `arg_value` only allows `IDENT.output` for var-refs, not the general
//   `IDENT.IDENT` form. Without this restriction, `a.b.c` would be
//   ambiguous (member-access vs. var-ref chain). Restricting var-refs to
//   the literal token `output` makes the parser zero-lookahead at every
//   decision point.
//
// Design: hand-written recursive descent (no parser generator dependency).
// Error reporting: anyhow with token position context.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::ast::*;
use super::lexer::{Lexer, Token};

pub fn parse(source: &str) -> anyhow::Result<EalProgram> {
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|e| anyhow::anyhow!(e))?;
    Parser::new(tokens).parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }
    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }
    fn peek_at(&self, offset: usize) -> &Token {
        self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof)
    }
    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        t
    }
    fn expect(&mut self, expected: &Token) -> anyhow::Result<()> {
        let t = self.advance();
        anyhow::ensure!(&t == expected, "expected {expected:?}, got {t:?}");
        Ok(())
    }
    fn expect_string(&mut self) -> anyhow::Result<String> {
        match self.advance() {
            Token::StringLit(s) => Ok(s),
            t => anyhow::bail!("expected string, got {t:?}"),
        }
    }
    fn expect_ident(&mut self) -> anyhow::Result<String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            t => anyhow::bail!("expected ident, got {t:?}"),
        }
    }
    fn expect_int(&mut self) -> anyhow::Result<i64> {
        match self.advance() {
            Token::IntLit(n) => Ok(n),
            t => anyhow::bail!("expected int, got {t:?}"),
        }
    }

    fn parse_program(&mut self) -> anyhow::Result<EalProgram> {
        let mission = self.parse_mission()?;
        Ok(EalProgram { mission })
    }

    fn parse_mission(&mut self) -> anyhow::Result<MissionDecl> {
        self.expect(&Token::Mission)?;
        let name = self.expect_string()?;
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while *self.peek() != Token::RBrace && *self.peek() != Token::Eof {
            stmts.push(self.parse_statement()?);
        }
        self.expect(&Token::RBrace)?;
        Ok(MissionDecl {
            name,
            statements: stmts,
        })
    }

    fn parse_statement(&mut self) -> anyhow::Result<Statement> {
        if *self.peek() == Token::Let {
            self.advance();
            let binding = self.expect_ident()?;
            self.expect(&Token::Eq)?;
            let call = self.parse_rhs()?;
            Ok(Statement::LetCall { binding, call })
        } else {
            Ok(Statement::Call(self.parse_rhs()?))
        }
    }

    /// Parse a call expression in either the traditional `call "..." on
    /// "..."` form or the member-call `agent.ability(args)` form. The
    /// branch is chosen by looking at the current token plus one
    /// lookahead — `Call` keyword takes the traditional path, an
    /// identifier followed by `.` takes the member-call path.
    fn parse_rhs(&mut self) -> anyhow::Result<CallExpr> {
        match self.peek() {
            Token::Call => self.parse_call_expr(),
            Token::Ident(_) if *self.peek_at(1) == Token::Dot => self.parse_member_call(),
            t => anyhow::bail!("expected `call ...` or `<agent>.<ability>(...)`, got {t:?}"),
        }
    }

    fn parse_call_expr(&mut self) -> anyhow::Result<CallExpr> {
        self.expect(&Token::Call)?;
        let function_name = self.expect_string()?;
        let target_node = if *self.peek() == Token::On {
            self.advance();
            Some(self.expect_string()?)
        } else {
            None
        };
        let arguments = if *self.peek() == Token::With {
            self.advance();
            self.expect(&Token::LBrace)?;
            let fields = self.parse_fields()?;
            self.expect(&Token::RBrace)?;
            fields
        } else {
            Vec::new()
        };
        let options = self.parse_options()?;
        // Traditional `call X on Y` form lowers to a Device target.
        // To call an agent, use member-call syntax (`agent.ability(...)`).
        Ok(CallExpr {
            function_name,
            target_node,
            target_kind: TargetKind::Device,
            arguments,
            options,
        })
    }

    /// Parse a member-call form: `<agent>.<ability>(name: value, ...)`.
    /// Lowers to a `CallExpr` with `target_kind = TargetKind::Agent`.
    /// The planner uses `target_kind` to choose the IR target variant
    /// (`IrTarget::Agent` here vs `IrTarget::Device` for the
    /// traditional form). The runtime dispatcher matches the resolved
    /// `IrTarget` and never re-classifies based on names.
    fn parse_member_call(&mut self) -> anyhow::Result<CallExpr> {
        let agent = self.expect_ident()?;
        self.expect(&Token::Dot)?;
        let ability = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let arguments = self.parse_named_args()?;
        self.expect(&Token::RParen)?;
        let options = self.parse_options()?;
        Ok(CallExpr {
            function_name: ability,
            target_node: Some(agent),
            target_kind: TargetKind::Agent,
            arguments,
            options,
        })
    }

    /// Parse a comma-separated `key: value` list inside `(...)`.
    /// `arg_value` is restricted to scalar literals or `IDENT.output`
    /// (the var-ref form) — see grammar ambiguity guard in the file
    /// header.
    fn parse_named_args(&mut self) -> anyhow::Result<Vec<Field>> {
        let mut fields = Vec::new();
        while *self.peek() != Token::RParen && *self.peek() != Token::Eof {
            let key = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let value = self.parse_arg_value()?;
            fields.push(Field { key, value });
            if *self.peek() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(fields)
    }

    /// Parse one argument value inside a member-call's named-arg list.
    /// Distinct from `parse_value` (which handles `with { ... }` blocks)
    /// only because it must reject the bare-identifier-as-string
    /// fallback that `parse_value` allows — bare idents in a member call
    /// are reserved for future use (e.g. references), so we make them an
    /// error today rather than silently coerce them to strings.
    ///
    /// Implementation note: the match consumes the token via `advance()`
    /// in a single step per arm and destructures the payload at the
    /// same point. The earlier `peek-then-if-let-advance-else-unreachable`
    /// form was load-bearing on the invariant that `peek()` and
    /// `advance()` see the same token; any future change that interposes
    /// whitespace skipping or similar between them would panic in an
    /// `unreachable!()` arm. Destructuring inside `match self.advance()`
    /// eliminates the invariant entirely.
    fn parse_arg_value(&mut self) -> anyhow::Result<FieldValue> {
        // For the IDENT case we need look-ahead (to see `.output`), so
        // branch before consuming.
        if matches!(self.peek(), Token::Ident(_)) {
            // Only `IDENT.output` is accepted here. Anything else is
            // an error — the grammar ambiguity guard requires that
            // bare idents not be silently coerced to strings.
            let ident = self.expect_ident()?;
            if *self.peek() == Token::Dot {
                self.advance();
                let acc = self.expect_ident()?;
                anyhow::ensure!(
                    acc == "output",
                    "unknown accessor '.{acc}' (only '.output' supported in member-call args)"
                );
                return Ok(FieldValue::VarRef { var_name: ident });
            }
            anyhow::bail!(
                "bare identifier '{ident}' is not a valid argument value; \
                 use a string literal, number, bool, or '<var>.output'"
            );
        }
        // Scalar literals: consume and destructure in one step.
        match self.advance() {
            Token::StringLit(s) => Ok(FieldValue::String(s)),
            Token::IntLit(n) => Ok(FieldValue::Int(n)),
            Token::FloatLit(f) => Ok(FieldValue::Float(f)),
            Token::BoolLit(b) => Ok(FieldValue::Bool(b)),
            t => anyhow::bail!("expected argument value, got {t:?}"),
        }
    }

    fn parse_fields(&mut self) -> anyhow::Result<Vec<Field>> {
        let mut fields = Vec::new();
        while *self.peek() != Token::RBrace && *self.peek() != Token::Eof {
            let key = self.expect_ident()?;
            self.expect(&Token::Eq)?;
            let value = self.parse_value()?;
            fields.push(Field { key, value });
            if *self.peek() == Token::Comma {
                self.advance();
            }
        }
        Ok(fields)
    }

    /// Parse one value inside a `with { ... }` field block.
    ///
    /// Unlike `parse_arg_value` (member-call named args), this form
    /// silently coerces a bare identifier to a string literal for
    /// legacy ergonomics with the traditional `call "..." on "..." with
    /// { key = value }` production. Everything else — var-refs, scalars
    /// — behaves identically.
    ///
    /// See `parse_arg_value` for the destructure-in-one-step rationale.
    fn parse_value(&mut self) -> anyhow::Result<FieldValue> {
        if matches!(self.peek(), Token::Ident(_)) {
            let ident = self.expect_ident()?;
            if *self.peek() == Token::Dot {
                self.advance();
                let acc = self.expect_ident()?;
                anyhow::ensure!(
                    acc == "output",
                    "unknown accessor '.{acc}' (only '.output' supported)"
                );
                return Ok(FieldValue::VarRef { var_name: ident });
            }
            return Ok(FieldValue::String(ident));
        }
        match self.advance() {
            Token::StringLit(s) => Ok(FieldValue::String(s)),
            Token::IntLit(n) => Ok(FieldValue::Int(n)),
            Token::FloatLit(f) => Ok(FieldValue::Float(f)),
            Token::BoolLit(b) => Ok(FieldValue::Bool(b)),
            t => anyhow::bail!("expected value, got {t:?}"),
        }
    }

    fn parse_options(&mut self) -> anyhow::Result<StepOptions> {
        let mut opts = StepOptions::default();
        loop {
            match self.peek() {
                Token::Timeout => {
                    self.advance();
                    opts.timeout_seconds = Some(self.expect_i32_in_range("timeout")?);
                }
                Token::Retries => {
                    self.advance();
                    opts.max_retries = Some(self.expect_i32_in_range("retries")?);
                }
                Token::OnFailure => {
                    self.advance();
                    opts.on_failure = Some(self.parse_policy()?);
                }
                Token::Optional => {
                    self.advance();
                    opts.optional = true;
                }
                _ => break,
            }
        }
        Ok(opts)
    }

    /// Read an integer literal and bounds-check it as a non-negative i32.
    /// The previous `expect_int()? as i32` silently truncated values
    /// outside `[i32::MIN, i32::MAX]` (e.g. `timeout 5_000_000_000`
    /// wrapped to `705_032_704`); negative values silently flowed
    /// through to the runtime where they meant the opposite of intent.
    /// Both `timeout` and `retries` are non-negative by definition, so
    /// we reject the negative range here too rather than hand garbage
    /// to the planner.
    fn expect_i32_in_range(&mut self, field: &str) -> anyhow::Result<i32> {
        let n = self.expect_int()?;
        if !(0..=i32::MAX as i64).contains(&n) {
            anyhow::bail!("{field} value {n} out of range (must be 0..={})", i32::MAX);
        }
        Ok(n as i32)
    }

    fn parse_policy(&mut self) -> anyhow::Result<FailurePolicy> {
        match self.advance() {
            Token::Abort => Ok(FailurePolicy::Abort),
            Token::Skip => Ok(FailurePolicy::Skip),
            Token::Retry => Ok(FailurePolicy::Retry),
            Token::Continue => Ok(FailurePolicy::Continue),
            t => anyhow::bail!("expected failure policy, got {t:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let p = parse(r#"mission "t" { let x = call "foo" on "n1" timeout 30 }"#).unwrap();
        assert_eq!(p.mission.name, "t");
        assert_eq!(p.mission.statements.len(), 1);
    }

    #[test]
    fn parse_var_ref() {
        let p = parse(r#"mission "t" { let a = call "x" on "n" let b = call "y" on "n" with { input = a.output } }"#).unwrap();
        assert_eq!(p.mission.statements.len(), 2);
    }

    #[test]
    fn parse_optional() {
        let p = parse(r#"mission "t" { call "ping" on "m" optional }"#).unwrap();
        match &p.mission.statements[0] {
            Statement::Call(c) => assert!(c.options.optional),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_rejects_oversize_timeout() {
        // The previous `as i32` truncated values >i32::MAX into garbage
        // (often negative) and silently passed them to the planner. The
        // new bounds check surfaces them as a parse error so the user
        // sees the problem at compile time, not as a mysterious negative
        // timeout at runtime.
        let err = parse(r#"mission "t" { call "x" on "n" timeout 5000000000 }"#).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("timeout") && msg.contains("out of range"),
            "expected an out-of-range error mentioning 'timeout', got: {msg}"
        );
    }

    #[test]
    fn parse_rejects_negative_retries() {
        // Negative retry counts are nonsensical and previously survived
        // the implicit `as i32` cast unchanged. Reject them at parse
        // time so the planner never has to defend against impossible
        // states.
        let err = parse(r#"mission "t" { call "x" on "n" retries -1 }"#).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("retries") && msg.contains("out of range"),
            "expected an out-of-range error mentioning 'retries', got: {msg}"
        );
    }

    #[test]
    fn parse_accepts_valid_timeout_and_retries_at_boundaries() {
        // 0 (unset/no-timeout) and i32::MAX must both still parse —
        // the bounds check rejects only values outside `0..=i32::MAX`.
        assert!(parse(r#"mission "t" { call "x" on "n" timeout 0 }"#).is_ok());
        assert!(parse(&format!(
            r#"mission "t" {{ call "x" on "n" timeout {} }}"#,
            i32::MAX
        ))
        .is_ok());
        assert!(parse(r#"mission "t" { call "x" on "n" retries 0 }"#).is_ok());
    }

    // ── Member-call form (ontology §6.2 surface) ───────────────────────────

    fn extract_call(p: &EalProgram, idx: usize) -> &CallExpr {
        match &p.mission.statements[idx] {
            Statement::LetCall { call, .. } => call,
            Statement::Call(c) => c,
        }
    }

    #[test]
    fn member_call_minimal() {
        let p = parse(r#"mission "t" { let r = claude.chat(prompt: "hi") }"#).unwrap();
        let c = extract_call(&p, 0);
        assert_eq!(c.function_name, "chat");
        assert_eq!(c.target_node.as_deref(), Some("claude"));
        assert_eq!(c.arguments.len(), 1);
        assert_eq!(c.arguments[0].key, "prompt");
        match &c.arguments[0].value {
            FieldValue::String(s) => assert_eq!(s, "hi"),
            v => panic!("expected string, got {v:?}"),
        }
    }

    #[test]
    fn member_call_no_args() {
        let p = parse(r#"mission "t" { let r = claude.ping() }"#).unwrap();
        let c = extract_call(&p, 0);
        assert_eq!(c.function_name, "ping");
        assert_eq!(c.target_node.as_deref(), Some("claude"));
        assert!(c.arguments.is_empty());
    }

    #[test]
    fn member_call_multi_args() {
        let p = parse(
            r#"mission "t" { let r = claude.review(file: "x.rs", strict: true, lines: 42) }"#,
        )
        .unwrap();
        let c = extract_call(&p, 0);
        assert_eq!(c.function_name, "review");
        assert_eq!(c.arguments.len(), 3);
        assert_eq!(c.arguments[0].key, "file");
        assert_eq!(c.arguments[1].key, "strict");
        assert_eq!(c.arguments[2].key, "lines");
        match &c.arguments[1].value {
            FieldValue::Bool(b) => assert!(*b),
            v => panic!("expected bool, got {v:?}"),
        }
        match &c.arguments[2].value {
            FieldValue::Int(n) => assert_eq!(*n, 42),
            v => panic!("expected int, got {v:?}"),
        }
    }

    #[test]
    fn member_call_with_options() {
        let p = parse(r#"mission "t" { let r = claude.chat(prompt: "hi") timeout 30 }"#).unwrap();
        let c = extract_call(&p, 0);
        assert_eq!(c.options.timeout_seconds, Some(30));
    }

    #[test]
    fn member_call_var_ref() {
        let p = parse(
            r#"mission "t" {
                let prev = claude.draft(topic: "x")
                let r = claude.review(input: prev.output)
            }"#,
        )
        .unwrap();
        let c = extract_call(&p, 1);
        assert_eq!(c.function_name, "review");
        match &c.arguments[0].value {
            FieldValue::VarRef { var_name } => assert_eq!(var_name, "prev"),
            v => panic!("expected var-ref, got {v:?}"),
        }
    }

    #[test]
    fn traditional_call_unchanged() {
        let p = parse(r#"mission "t" { let r = call "chat" on "claude" with { prompt = "hi" } }"#)
            .unwrap();
        let c = extract_call(&p, 0);
        assert_eq!(c.function_name, "chat");
        assert_eq!(c.target_node.as_deref(), Some("claude"));
        assert_eq!(c.arguments.len(), 1);
    }

    #[test]
    fn member_call_rejects_bare_ident_as_value() {
        // Bare identifiers in named-arg position are rejected to keep the
        // grammar zero-lookahead — we don't want a future feature creep
        // to accidentally make `(prompt: foo)` resolve as a string.
        let r = parse(r#"mission "t" { let r = claude.chat(prompt: foo) }"#);
        assert!(r.is_err(), "bare identifier should be rejected");
    }
}
