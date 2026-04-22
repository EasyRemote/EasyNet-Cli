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
        match self.peek() {
            Token::Let => {
                self.advance();
                let binding = self.expect_ident()?;
                self.expect(&Token::Eq)?;
                let call = self.parse_rhs()?;
                Ok(Statement::LetCall { binding, call })
            }
            // PR-10 control-flow blocks. `loop` is a reserved token;
            // `chat` and `handoff` are contextual keywords — their
            // statement-at-start shape disambiguates from a
            // member-call because the next token after a block
            // introducer is never `.` or `(`.
            Token::Loop => Ok(Statement::Loop(self.parse_loop_block()?)),
            Token::Ident(s) if s == "chat" && !self.is_member_call_start() => {
                Ok(Statement::Chat(self.parse_chat_block()?))
            }
            Token::Ident(s) if s == "handoff" && !self.is_member_call_start() => {
                Ok(Statement::Handoff(self.parse_handoff_block()?))
            }
            _ => Ok(Statement::Call(self.parse_rhs()?)),
        }
    }

    /// True when the next two tokens look like a member-call
    /// `<agent>.<ability>(...)` or a grouped ability target `<agent>(...)`.
    /// Used to keep `chat`/`handoff` from swallowing an agent named
    /// `chat` at statement start.
    fn is_member_call_start(&self) -> bool {
        matches!(self.peek_at(1), Token::Dot | Token::LParen)
    }

    /// `loop "<name>?" max_iters: N { body { … } verify { … } }`
    ///
    /// The body and verify blocks are statement lists. Nested loops
    /// are syntactically legal; the planner enforces the
    /// compile-time recursion-depth cap (RFC §4.2).
    fn parse_loop_block(&mut self) -> anyhow::Result<LoopBlock> {
        self.expect(&Token::Loop)?;
        let name = self.parse_optional_block_name()?;
        // Header attribute: `max_iters: <int>` (contextual keyword).
        self.expect_contextual_ident("max_iters")?;
        self.expect(&Token::Colon)?;
        let max_iters = self.expect_u32()?;
        self.expect(&Token::LBrace)?;
        let mut body: Vec<Statement> = Vec::new();
        let mut verify: Vec<Statement> = Vec::new();
        // Sub-blocks arrive in source order. `body` must appear
        // exactly once; `verify` must appear exactly once. We check
        // the counts after the loop rather than inline so the user
        // sees "missing verify" instead of "unexpected RBrace".
        let mut saw_body = false;
        let mut saw_verify = false;
        while *self.peek() != Token::RBrace {
            match self.peek() {
                Token::Body => {
                    if saw_body {
                        anyhow::bail!("loop: duplicate `body` sub-block");
                    }
                    saw_body = true;
                    self.advance();
                    self.expect(&Token::LBrace)?;
                    while *self.peek() != Token::RBrace {
                        body.push(self.parse_statement()?);
                    }
                    self.expect(&Token::RBrace)?;
                }
                Token::Verify => {
                    if saw_verify {
                        anyhow::bail!("loop: duplicate `verify` sub-block");
                    }
                    saw_verify = true;
                    self.advance();
                    self.expect(&Token::LBrace)?;
                    while *self.peek() != Token::RBrace {
                        verify.push(self.parse_statement()?);
                    }
                    self.expect(&Token::RBrace)?;
                }
                t => anyhow::bail!(
                    "loop: expected `body` or `verify` sub-block, got {t:?}"
                ),
            }
        }
        self.expect(&Token::RBrace)?;
        if !saw_body {
            anyhow::bail!("loop: missing `body` sub-block");
        }
        if !saw_verify {
            anyhow::bail!("loop: missing `verify` sub-block");
        }
        Ok(LoopBlock {
            name,
            max_iters,
            body,
            verify,
        })
    }

    /// `chat "<name>?" participants: [A, B] max_turns: N { topic:? visibility:? }`
    ///
    /// `chat` is a contextual keyword — it arrives at this parser as
    /// `Token::Ident("chat")` only when the statement-level
    /// disambiguator confirmed it is not a member-call.
    fn parse_chat_block(&mut self) -> anyhow::Result<ChatBlock> {
        // Consume the `chat` identifier.
        self.expect_contextual_ident("chat")?;
        let name = self.parse_optional_block_name()?;
        // Header attrs before `{`. Order is flexible; both required.
        let mut participants: Option<Vec<String>> = None;
        let mut max_turns: Option<u32> = None;
        while *self.peek() != Token::LBrace {
            let attr = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            match attr.as_str() {
                "participants" => {
                    if participants.is_some() {
                        anyhow::bail!("chat: duplicate `participants:` header attribute");
                    }
                    participants = Some(self.parse_agent_name_list()?);
                }
                "max_turns" => {
                    if max_turns.is_some() {
                        anyhow::bail!("chat: duplicate `max_turns:` header attribute");
                    }
                    max_turns = Some(self.expect_u32()?);
                }
                other => anyhow::bail!(
                    "chat header: expected `participants:` or `max_turns:`, got `{other}:`"
                ),
            }
        }
        let participants = participants
            .ok_or_else(|| anyhow::anyhow!("chat: missing `participants:` header attribute"))?;
        let max_turns = max_turns
            .ok_or_else(|| anyhow::anyhow!("chat: missing `max_turns:` header attribute"))?;

        self.expect(&Token::LBrace)?;
        let mut topic: Option<String> = None;
        let mut visibility: Option<ChatVisibility> = None;
        while *self.peek() != Token::RBrace {
            let attr = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            match attr.as_str() {
                "topic" => {
                    if topic.is_some() {
                        anyhow::bail!("chat: duplicate `topic:` attribute");
                    }
                    topic = Some(self.expect_string()?);
                }
                "visibility" => {
                    if visibility.is_some() {
                        anyhow::bail!("chat: duplicate `visibility:` attribute");
                    }
                    let v = self.expect_ident()?;
                    visibility = Some(match v.as_str() {
                        "fan_out" => ChatVisibility::FanOut,
                        "round_robin" => ChatVisibility::RoundRobin,
                        other => anyhow::bail!(
                            "chat visibility: expected `fan_out` or `round_robin`, got `{other}`"
                        ),
                    });
                }
                other => anyhow::bail!(
                    "chat body: expected `topic:` or `visibility:`, got `{other}:`"
                ),
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(ChatBlock {
            name,
            participants,
            max_turns,
            topic,
            visibility,
        })
    }

    /// `handoff "<name>?" { from: A to: B context_mode:? prompt:? }`
    ///
    /// `handoff` is a contextual keyword; same rationale as `chat`.
    fn parse_handoff_block(&mut self) -> anyhow::Result<HandoffBlock> {
        self.expect_contextual_ident("handoff")?;
        let name = self.parse_optional_block_name()?;
        self.expect(&Token::LBrace)?;

        let mut from: Option<String> = None;
        let mut to: Option<String> = None;
        let mut context_mode: Option<HandoffContextMode> = None;
        let mut prompt: Option<String> = None;
        while *self.peek() != Token::RBrace {
            let attr = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            match attr.as_str() {
                "from" => {
                    if from.is_some() {
                        anyhow::bail!("handoff: duplicate `from:` attribute");
                    }
                    from = Some(self.expect_agent_ident()?);
                }
                "to" => {
                    if to.is_some() {
                        anyhow::bail!("handoff: duplicate `to:` attribute");
                    }
                    to = Some(self.expect_agent_ident()?);
                }
                "context_mode" => {
                    if context_mode.is_some() {
                        anyhow::bail!("handoff: duplicate `context_mode:` attribute");
                    }
                    let v = self.expect_ident()?;
                    context_mode = Some(match v.as_str() {
                        "full" => HandoffContextMode::Full,
                        "summary" => HandoffContextMode::Summary,
                        "none" => HandoffContextMode::None,
                        other => anyhow::bail!(
                            "handoff context_mode: expected `full` / `summary` / `none`, got `{other}`"
                        ),
                    });
                }
                "prompt" => {
                    if prompt.is_some() {
                        anyhow::bail!("handoff: duplicate `prompt:` attribute");
                    }
                    prompt = Some(self.expect_string()?);
                }
                other => anyhow::bail!(
                    "handoff: expected `from:` / `to:` / `context_mode:` / `prompt:`, got `{other}:`"
                ),
            }
        }
        self.expect(&Token::RBrace)?;
        let from = from.ok_or_else(|| anyhow::anyhow!("handoff: missing `from:`"))?;
        let to = to.ok_or_else(|| anyhow::anyhow!("handoff: missing `to:`"))?;
        Ok(HandoffBlock {
            name,
            from,
            to,
            context_mode,
            prompt,
        })
    }

    /// Consume an `Ident(s)` token whose value equals the expected
    /// string. Used for contextual keywords that are not reserved at
    /// the lexer level.
    fn expect_contextual_ident(&mut self, expected: &str) -> anyhow::Result<()> {
        match self.peek().clone() {
            Token::Ident(s) if s == expected => {
                self.advance();
                Ok(())
            }
            t => anyhow::bail!("expected identifier `{expected}`, got {t:?}"),
        }
    }

    /// Parse an optional block name. If the next token is a string
    /// literal, consume it as the block's name. Otherwise return
    /// `None`. Block names are exported as `<name>.result` /
    /// `<name>.transcript` to the enclosing scope; anonymous blocks
    /// export nothing.
    fn parse_optional_block_name(&mut self) -> anyhow::Result<Option<String>> {
        if let Token::StringLit(_) = self.peek() {
            let s = self.expect_string()?;
            Ok(Some(s))
        } else {
            Ok(None)
        }
    }

    /// Parse a `[A, B, "c/d"]` style list of agent names. Each entry
    /// may be a bare identifier (short form) or a string literal
    /// (full tenant/name form). The planner resolves each via
    /// `AgentId::parse` later.
    fn parse_agent_name_list(&mut self) -> anyhow::Result<Vec<String>> {
        self.expect(&Token::LBracket)?;
        let mut out: Vec<String> = Vec::new();
        while *self.peek() != Token::RBracket {
            out.push(self.expect_agent_ident()?);
            match self.peek() {
                Token::Comma => {
                    self.advance();
                }
                Token::RBracket => {}
                t => anyhow::bail!("agent list: expected `,` or `]`, got {t:?}"),
            }
        }
        self.expect(&Token::RBracket)?;
        Ok(out)
    }

    /// Agent reference in a block header: either a bare identifier
    /// (`claude`) or a string literal (`"tenant/claude"`).
    fn expect_agent_ident(&mut self) -> anyhow::Result<String> {
        match self.peek() {
            Token::StringLit(_) => self.expect_string(),
            Token::Ident(_) => self.expect_ident(),
            t => anyhow::bail!("expected agent name (bare ident or string), got {t:?}"),
        }
    }

    /// Expect a non-negative integer literal that fits in u32. Used
    /// for `max_iters` / `max_turns`.
    fn expect_u32(&mut self) -> anyhow::Result<u32> {
        match self.peek().clone() {
            Token::IntLit(n) => {
                self.advance();
                if n < 0 {
                    anyhow::bail!("expected non-negative integer, got {n}");
                }
                u32::try_from(n).map_err(|_| anyhow::anyhow!("integer {n} out of u32 range"))
            }
            t => anyhow::bail!("expected integer literal, got {t:?}"),
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
            s => panic!(
                "extract_call expected a flat Call statement, got block variant: {s:?}"
            ),
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

    // ── PR-10 control-flow blocks (RFC §3) ─────────────────────────────────

    /// Helper: the block variant extractor, symmetric to
    /// `extract_call`. Returns `None` if the statement at `idx` is
    /// not the expected block type.
    fn as_loop(p: &EalProgram, idx: usize) -> Option<&LoopBlock> {
        match &p.mission.statements[idx] {
            Statement::Loop(l) => Some(l),
            _ => None,
        }
    }
    fn as_chat(p: &EalProgram, idx: usize) -> Option<&ChatBlock> {
        match &p.mission.statements[idx] {
            Statement::Chat(c) => Some(c),
            _ => None,
        }
    }
    fn as_handoff(p: &EalProgram, idx: usize) -> Option<&HandoffBlock> {
        match &p.mission.statements[idx] {
            Statement::Handoff(h) => Some(h),
            _ => None,
        }
    }

    #[test]
    fn loop_block_minimal_with_name() {
        let src = r#"
            mission "t" {
                loop "review" max_iters: 4 {
                    body {
                        let r = reviewer.review(artifacts: "src/")
                    }
                    verify {
                        reviewer.rule_on(data: r.output)
                    }
                }
            }"#;
        let p = parse(src).unwrap();
        let l = as_loop(&p, 0).expect("loop at index 0");
        assert_eq!(l.name.as_deref(), Some("review"));
        assert_eq!(l.max_iters, 4);
        assert_eq!(l.body.len(), 1);
        assert_eq!(l.verify.len(), 1);
    }

    #[test]
    fn loop_block_anonymous_parses() {
        // Anonymous loop (no name string before `max_iters:`).
        // Exports no binding; useful for fire-and-forget iteration.
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { let r = a.chat(prompt: "hi") }
                    verify { a.check(of: r.output) }
                }
            }"#;
        let p = parse(src).unwrap();
        let l = as_loop(&p, 0).expect("anonymous loop");
        assert!(l.name.is_none());
        assert_eq!(l.max_iters, 2);
    }

    #[test]
    fn loop_block_rejects_duplicate_body() {
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { let r = a.chat(prompt: "x") }
                    body { let r = a.chat(prompt: "y") }
                    verify { a.check(of: r.output) }
                }
            }"#;
        let err = parse(src).unwrap_err().to_string();
        assert!(err.contains("duplicate `body`"), "got: {err}");
    }

    #[test]
    fn loop_block_rejects_missing_verify() {
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { let r = a.chat(prompt: "x") }
                }
            }"#;
        let err = parse(src).unwrap_err().to_string();
        assert!(err.contains("missing `verify`"), "got: {err}");
    }

    #[test]
    fn chat_block_with_participants_and_topic() {
        let src = r#"
            mission "t" {
                chat "triangulate" participants: [claude, codex] max_turns: 3 {
                    topic: "What's wrong?"
                    visibility: fan_out
                }
            }"#;
        let p = parse(src).unwrap();
        let c = as_chat(&p, 0).expect("chat at index 0");
        assert_eq!(c.name.as_deref(), Some("triangulate"));
        assert_eq!(c.participants, vec!["claude", "codex"]);
        assert_eq!(c.max_turns, 3);
        assert_eq!(c.topic.as_deref(), Some("What's wrong?"));
        assert_eq!(c.visibility, Some(ChatVisibility::FanOut));
    }

    #[test]
    fn chat_block_defaults_visibility_to_none_in_ast() {
        // RFC §3.2 says fan_out is the default; the parser records
        // the explicit presence only — the planner applies the
        // default when building IR. Test documents that policy:
        // AST visibility stays `None` until the planner.
        let src = r#"
            mission "t" {
                chat participants: [a, b] max_turns: 1 {}
            }"#;
        let p = parse(src).unwrap();
        let c = as_chat(&p, 0).unwrap();
        assert!(c.visibility.is_none());
        assert!(c.topic.is_none());
    }

    #[test]
    fn chat_block_rejects_duplicate_participants() {
        let src = r#"
            mission "t" {
                chat participants: [a] participants: [b] max_turns: 1 {}
            }"#;
        let err = parse(src).unwrap_err().to_string();
        assert!(err.contains("duplicate `participants:`"), "got: {err}");
    }

    #[test]
    fn chat_block_accepts_round_robin_visibility() {
        let src = r#"
            mission "t" {
                chat participants: [a, b] max_turns: 1 {
                    visibility: round_robin
                }
            }"#;
        let p = parse(src).unwrap();
        assert_eq!(
            as_chat(&p, 0).unwrap().visibility,
            Some(ChatVisibility::RoundRobin)
        );
    }

    #[test]
    fn handoff_block_full_mode() {
        let src = r#"
            mission "t" {
                handoff {
                    from: claude
                    to: codex
                    context_mode: summary
                    prompt: "Finish the plan"
                }
            }"#;
        let p = parse(src).unwrap();
        let h = as_handoff(&p, 0).expect("handoff at index 0");
        assert_eq!(h.from, "claude");
        assert_eq!(h.to, "codex");
        assert_eq!(h.context_mode, Some(HandoffContextMode::Summary));
        assert_eq!(h.prompt.as_deref(), Some("Finish the plan"));
    }

    #[test]
    fn handoff_block_accepts_none_context_mode() {
        let src = r#"
            mission "t" {
                handoff { from: a to: b context_mode: none }
            }"#;
        let p = parse(src).unwrap();
        let h = as_handoff(&p, 0).unwrap();
        assert_eq!(h.context_mode, Some(HandoffContextMode::None));
    }

    #[test]
    fn handoff_block_rejects_missing_from() {
        let src = r#"
            mission "t" {
                handoff { to: b }
            }"#;
        let err = parse(src).unwrap_err().to_string();
        assert!(err.contains("missing `from:`"), "got: {err}");
    }
}
