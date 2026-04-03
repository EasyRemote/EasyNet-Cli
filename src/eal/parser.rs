// EasyNet CLI — EAL Parser
// ========================
//
// File: src/eal/parser.rs
// Description: Recursive descent parser — converts token stream to EalProgram AST.
//
// Grammar (EBNF):
//   program    = mission_decl
//   mission    = "mission" STRING "{" statement* "}"
//   statement  = "let" IDENT "=" call_expr | call_expr
//   call_expr  = "call" STRING ("on" STRING)? ("with" "{" field_list "}")? option*
//   option     = "timeout" INT | "retries" INT | "on_failure" POLICY | "optional"
//   field      = IDENT "=" (STRING | INT | FLOAT | BOOL | IDENT "." "output")
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
            let call = self.parse_call_expr()?;
            Ok(Statement::LetCall { binding, call })
        } else {
            Ok(Statement::Call(self.parse_call_expr()?))
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
        Ok(CallExpr { function_name, target_node, arguments, options })
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
}
