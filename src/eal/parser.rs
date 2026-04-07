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
    let tokens = Lexer::new(source).tokenize().map_err(|e| anyhow::anyhow!(e))?;
    Parser::new(tokens).parse_program()
}

struct Parser { tokens: Vec<Token>, pos: usize }

impl Parser {
    fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }
    fn peek(&self) -> &Token { self.tokens.get(self.pos).unwrap_or(&Token::Eof) }
    fn peek_at(&self, offset: usize) -> &Token { self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof) }
    fn advance(&mut self) -> Token { let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof); self.pos += 1; t }
    fn expect(&mut self, expected: &Token) -> anyhow::Result<()> {
        let t = self.advance();
        anyhow::ensure!(&t == expected, "expected {expected:?}, got {t:?}");
        Ok(())
    }
    fn expect_string(&mut self) -> anyhow::Result<String> { match self.advance() { Token::StringLit(s) => Ok(s), t => anyhow::bail!("expected string, got {t:?}") } }
    fn expect_ident(&mut self) -> anyhow::Result<String> { match self.advance() { Token::Ident(s) => Ok(s), t => anyhow::bail!("expected ident, got {t:?}") } }
    fn expect_int(&mut self) -> anyhow::Result<i64> { match self.advance() { Token::IntLit(n) => Ok(n), t => anyhow::bail!("expected int, got {t:?}") } }

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
        Ok(MissionDecl { name, statements: stmts })
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
            t => anyhow::bail!(
                "expected `call ...` or `<agent>.<ability>(...)`, got {t:?}"
            ),
        }
    }

    fn parse_call_expr(&mut self) -> anyhow::Result<CallExpr> {
        self.expect(&Token::Call)?;
        let function_name = self.expect_string()?;
        let target_node = if *self.peek() == Token::On { self.advance(); Some(self.expect_string()?) } else { None };
        let arguments = if *self.peek() == Token::With {
            self.advance(); self.expect(&Token::LBrace)?;
            let fields = self.parse_fields()?;
            self.expect(&Token::RBrace)?; fields
        } else { Vec::new() };
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
    fn parse_arg_value(&mut self) -> anyhow::Result<FieldValue> {
        match self.peek().clone() {
            Token::StringLit(_) => {
                if let Token::StringLit(s) = self.advance() { Ok(FieldValue::String(s)) } else { unreachable!() }
            }
            Token::IntLit(_) => {
                if let Token::IntLit(n) = self.advance() { Ok(FieldValue::Int(n)) } else { unreachable!() }
            }
            Token::FloatLit(_) => {
                if let Token::FloatLit(f) = self.advance() { Ok(FieldValue::Float(f)) } else { unreachable!() }
            }
            Token::BoolLit(_) => {
                if let Token::BoolLit(b) = self.advance() { Ok(FieldValue::Bool(b)) } else { unreachable!() }
            }
            Token::Ident(_) => {
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
                    Ok(FieldValue::VarRef { var_name: ident })
                } else {
                    anyhow::bail!(
                        "bare identifier '{ident}' is not a valid argument value; \
                         use a string literal, number, bool, or '<var>.output'"
                    );
                }
            }
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
            if *self.peek() == Token::Comma { self.advance(); }
        }
        Ok(fields)
    }

    fn parse_value(&mut self) -> anyhow::Result<FieldValue> {
        match self.peek().clone() {
            Token::StringLit(_) => { if let Token::StringLit(s) = self.advance() { Ok(FieldValue::String(s)) } else { unreachable!() } }
            Token::IntLit(_) => { if let Token::IntLit(n) = self.advance() { Ok(FieldValue::Int(n)) } else { unreachable!() } }
            Token::FloatLit(_) => { if let Token::FloatLit(f) = self.advance() { Ok(FieldValue::Float(f)) } else { unreachable!() } }
            Token::BoolLit(_) => { if let Token::BoolLit(b) = self.advance() { Ok(FieldValue::Bool(b)) } else { unreachable!() } }
            Token::Ident(_) => {
                let ident = self.expect_ident()?;
                if *self.peek() == Token::Dot {
                    self.advance();
                    let acc = self.expect_ident()?;
                    anyhow::ensure!(acc == "output", "unknown accessor '.{acc}' (only '.output' supported)");
                    Ok(FieldValue::VarRef { var_name: ident })
                } else {
                    Ok(FieldValue::String(ident))
                }
            }
            t => anyhow::bail!("expected value, got {t:?}"),
        }
    }

    fn parse_options(&mut self) -> anyhow::Result<StepOptions> {
        let mut opts = StepOptions::default();
        loop {
            match self.peek() {
                Token::Timeout => { self.advance(); opts.timeout_seconds = Some(self.expect_int()? as i32); }
                Token::Retries => { self.advance(); opts.max_retries = Some(self.expect_int()? as i32); }
                Token::OnFailure => { self.advance(); opts.on_failure = Some(self.parse_policy()?); }
                Token::Optional => { self.advance(); opts.optional = true; }
                _ => break,
            }
        }
        Ok(opts)
    }

    fn parse_policy(&mut self) -> anyhow::Result<FailurePolicy> {
        match self.advance() {
            Token::Abort => Ok(FailurePolicy::Abort), Token::Skip => Ok(FailurePolicy::Skip),
            Token::Retry => Ok(FailurePolicy::Retry), Token::Continue => Ok(FailurePolicy::Continue),
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
        match &p.mission.statements[0] { Statement::Call(c) => assert!(c.options.optional), _ => panic!() }
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
        let p = parse(
            r#"mission "t" { let r = claude.chat(prompt: "hi") timeout 30 }"#,
        )
        .unwrap();
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
        let p = parse(
            r#"mission "t" { let r = call "chat" on "claude" with { prompt = "hi" } }"#,
        )
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
