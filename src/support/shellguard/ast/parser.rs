// EasyNet CLI — ShellGuard: tree-sitter-bash parser wrapper
// =========================================================
//
// File: src/support/shellguard/ast/parser.rs
// Description: Owns the tree-sitter `Parser` lifecycle. Each
//              call to [`parse_root`] runs in its own thread-local
//              parser (the `Parser` itself is not `Send`/`Sync`),
//              loads the bash grammar once per thread, and
//              returns a `Tree` ready for the walker.
//
// Why thread-local instead of `LazyLock<Mutex<Parser>>`
// -----------------------------------------------------
// Two options:
//
//   A. One process-wide `Mutex<Parser>` behind `LazyLock`.
//   B. One `Parser` per thread via `thread_local!`.
//
// (A) serialises parsing across the entire CLI — the daemon
// processes shell.run from many simultaneous InvokeBidi
// channels, and a global mutex would funnel them through one
// parser. tree-sitter parses are cheap (<1 ms for typical
// commands) but a few thousand per second under contention
// would still measurably hurt latency.
//
// (B) avoids the lock and pays only the per-thread initial
// `set_language` cost (~microseconds). The `Parser` lives until
// the thread exits. This matches AliveCode's WASM
// `ensureInitialized()` pattern (one parser per V8 isolate)
// and is the documented tree-sitter recommendation for
// concurrent parsing.
//
// Footnote on `set_language` failure
// ----------------------------------
// `set_language` returns `LanguageError` when the grammar's
// ABI version doesn't match the parser library — a build-time
// invariant violation, never observable in shipped binaries
// (we pin both crates in `Cargo.toml`). We surface it as a
// distinct `ParseError::LanguageLoad` so a future bump of one
// dep without the other is loud at the call site rather than
// silently producing `parse-unavailable`.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::cell::RefCell;
use std::error::Error;
use std::fmt;

use tree_sitter::{Parser, Tree};

/// Distinct failure modes for [`parse_root`].
///
/// `LanguageLoad` means the bash grammar refused to bind to a
/// `Parser` — should be impossible at runtime given the pinned
/// dep set. `ParseFailed` means tree-sitter returned `None` from
/// `Parser::parse`, which the docs call out as "out of memory or
/// cancelled" — also unreachable in our usage (no cancellation,
/// no resource limits set).
///
/// Either error routes the caller to `ParseUnavailable`.
#[derive(Debug)]
pub enum ParseError {
    LanguageLoad(tree_sitter::LanguageError),
    ParseFailed,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LanguageLoad(e) => write!(f, "tree-sitter-bash language load failed: {e}"),
            Self::ParseFailed => write!(f, "tree-sitter parse returned None"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LanguageLoad(e) => Some(e),
            Self::ParseFailed => None,
        }
    }
}

thread_local! {
    /// Per-thread parser instance. Lazily constructed on first use,
    /// re-used for the lifetime of the thread. `RefCell` because
    /// `Parser::parse` takes `&mut self` and we want to share the
    /// instance across calls without re-loading the grammar each
    /// time.
    static PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

/// Parse `cmd` as bash and return the resulting `Tree`. The
/// caller must keep `cmd` alive for the duration of any walker
/// usage of the tree's nodes — `Node::utf8_text` borrows from
/// the source slice the parser was given.
///
/// Empty input is rejected by tree-sitter (returns a `program`
/// node with no children); we forward that to the walker, which
/// returns `Simple { commands: [] }` for it. No special-cased
/// short-circuit here, so the source-of-truth for empty handling
/// stays in one place.
pub fn parse_root(cmd: &str) -> Result<Tree, ParseError> {
    PARSER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let mut p = Parser::new();
            p.set_language(&tree_sitter_bash::LANGUAGE.into())
                .map_err(ParseError::LanguageLoad)?;
            *slot = Some(p);
        }
        let parser = slot.as_mut().expect("parser slot just initialised");
        parser.parse(cmd, None).ok_or(ParseError::ParseFailed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_string_to_program_with_no_commands() {
        let tree = parse_root("").expect("parse empty");
        let root = tree.root_node();
        assert_eq!(root.kind(), "program");
        assert_eq!(root.named_child_count(), 0);
    }

    #[test]
    fn parses_simple_command() {
        let tree = parse_root("ls -la").expect("parse simple");
        let root = tree.root_node();
        assert_eq!(root.kind(), "program");
        let cmd = root.named_child(0).expect("one command child");
        assert_eq!(cmd.kind(), "command");
    }

    #[test]
    fn parses_pipeline_node() {
        let tree = parse_root("ls | wc -l").unwrap();
        let pipeline = tree.root_node().named_child(0).unwrap();
        assert_eq!(pipeline.kind(), "pipeline");
    }

    #[test]
    fn parses_list_for_logical_and() {
        let tree = parse_root("a && b").unwrap();
        let list = tree.root_node().named_child(0).unwrap();
        assert_eq!(list.kind(), "list");
    }

    #[test]
    fn parses_redirected_statement() {
        let tree = parse_root("echo hi > out.txt").unwrap();
        let red = tree.root_node().named_child(0).unwrap();
        assert_eq!(red.kind(), "redirected_statement");
    }

    #[test]
    fn parses_command_substitution_node() {
        // We will reject this in the walker; the parser must still
        // produce a syntax-correct tree with a `command_substitution`
        // node so the walker has something to inspect.
        let tree = parse_root("echo $(date)").unwrap();
        let cmd = tree.root_node().named_child(0).unwrap();
        assert_eq!(cmd.kind(), "command");
        // Walking the command's children should reveal a
        // command_substitution somewhere — exact path depends on
        // tree-sitter-bash version, so just check existence.
        let mut found = false;
        let mut cursor = cmd.walk();
        for child in cmd.named_children(&mut cursor) {
            if child.kind() == "command_substitution" {
                found = true;
                break;
            }
        }
        assert!(found, "expected command_substitution in echo $(date)");
    }

    #[test]
    fn parser_is_reusable_across_calls() {
        // Stresses thread_local re-use: multiple parses on one
        // thread must not double-init the grammar.
        for src in ["ls", "true && false", "echo a | tac"] {
            assert!(parse_root(src).is_ok(), "{src} parse");
        }
    }
}
