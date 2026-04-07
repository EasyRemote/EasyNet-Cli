// EasyNet CLI — EAL Lexer
// =======================
//
// File: src/eal/lexer.rs
// Description: Tokenizer for the EAL language.
//
// Token Categories:
//   Keywords:    mission, let, call, on, with, timeout, retries, on_failure, optional
//   Policies:    abort, skip, retry, continue
//   Literals:    "string", 42, 3.14, true/false
//   Identifiers: variable names (e.g., photo, config)
//   Symbols:     { } ( ) = : , .
//   Comments:    // line comments (skipped during tokenization)
//
// Member-call syntax (added per ontology §6.2):
//   `let r = agent.ability(prompt: "hello")` requires `( )` for the
//   argument list and `:` for named-arg separators. The lexer treats them
//   as single-character symbol tokens identical in handling to the
//   existing `{ } = , .`.
//
// The lexer is context-free — keyword/identifier distinction is purely lexical.
// `output` is NOT a keyword; it's an identifier that the parser interprets as an accessor.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Mission, Let, Call, On, With, Timeout, Retries, OnFailure, Optional,
    Abort, Skip, Retry, Continue,
    StringLit(String), IntLit(i64), FloatLit(f64), BoolLit(bool),
    Ident(String),
    LBrace, RBrace, LParen, RParen, Eq, Colon, Comma, Dot,
    Eof,
}

pub struct Lexer<'a> { input: &'a [u8], pos: usize }

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self { Self { input: input.as_bytes(), pos: 0 } }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let done = tok == Token::Eof;
            tokens.push(tok);
            if done { break; }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_ws();
        if self.pos >= self.input.len() { return Ok(Token::Eof); }
        let ch = self.input[self.pos];
        match ch {
            b'{' => { self.pos += 1; Ok(Token::LBrace) }
            b'}' => { self.pos += 1; Ok(Token::RBrace) }
            b'(' => { self.pos += 1; Ok(Token::LParen) }
            b')' => { self.pos += 1; Ok(Token::RParen) }
            b'=' => { self.pos += 1; Ok(Token::Eq) }
            b':' => { self.pos += 1; Ok(Token::Colon) }
            b',' => { self.pos += 1; Ok(Token::Comma) }
            b'.' => { self.pos += 1; Ok(Token::Dot) }
            b'"' => self.read_string(),
            b'0'..=b'9' | b'-' => self.read_number(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.read_word(),
            _ => Err(format!("unexpected '{}' at {}", ch as char, self.pos)),
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch.is_ascii_whitespace() { self.pos += 1; }
            else if ch == b'/' && self.pos + 1 < self.input.len() && self.input[self.pos + 1] == b'/' {
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' { self.pos += 1; }
            } else { break; }
        }
    }

    fn read_string(&mut self) -> Result<Token, String> {
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos] != b'"' {
            if self.input[self.pos] == b'\\' { self.pos += 1; }
            self.pos += 1;
        }
        if self.pos >= self.input.len() { return Err("unterminated string".into()); }
        let s = String::from_utf8_lossy(&self.input[start..self.pos]).into_owned();
        self.pos += 1;
        Ok(Token::StringLit(s))
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let start = self.pos;
        if self.input[self.pos] == b'-' { self.pos += 1; }
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() { self.pos += 1; }
        let is_float = self.pos < self.input.len() && self.input[self.pos] == b'.'
            && self.pos + 1 < self.input.len() && self.input[self.pos + 1].is_ascii_digit();
        if is_float {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() { self.pos += 1; }
            let s = String::from_utf8_lossy(&self.input[start..self.pos]);
            Ok(Token::FloatLit(s.parse().map_err(|e| format!("{e}"))?))
        } else {
            let s = String::from_utf8_lossy(&self.input[start..self.pos]);
            Ok(Token::IntLit(s.parse().map_err(|e| format!("{e}"))?))
        }
    }

    fn read_word(&mut self) -> Result<Token, String> {
        let start = self.pos;
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_') {
            self.pos += 1;
        }
        let w = String::from_utf8_lossy(&self.input[start..self.pos]).into_owned();
        Ok(match w.as_str() {
            "mission" => Token::Mission, "let" => Token::Let, "call" => Token::Call,
            "on" => Token::On, "with" => Token::With, "timeout" => Token::Timeout,
            "retries" => Token::Retries, "on_failure" => Token::OnFailure,
            "optional" => Token::Optional, "abort" => Token::Abort, "skip" => Token::Skip,
            "retry" => Token::Retry, "continue" => Token::Continue,
            "true" => Token::BoolLit(true), "false" => Token::BoolLit(false),
            _ => Token::Ident(w),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_call_tokenization() {
        // `claude.chat(prompt: "hi")` must produce the exact token
        // sequence the parser's member_call branch expects.
        let mut lexer = Lexer::new(r#"claude.chat(prompt: "hi")"#);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("claude".to_string()),
                Token::Dot,
                Token::Ident("chat".to_string()),
                Token::LParen,
                Token::Ident("prompt".to_string()),
                Token::Colon,
                Token::StringLit("hi".to_string()),
                Token::RParen,
                Token::Eof,
            ],
        );
    }

    #[test]
    fn empty_paren_call() {
        let tokens = Lexer::new("claude.ping()").tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("claude".to_string()),
                Token::Dot,
                Token::Ident("ping".to_string()),
                Token::LParen,
                Token::RParen,
                Token::Eof,
            ],
        );
    }
}
