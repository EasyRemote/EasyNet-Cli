# MCP Child Stdio Bounded Framing

## Objective

Close the remaining MCP stdio memory-boundary fork on outbound child-server
reads. The daemon already bounds its local MCP stdin line reader; the child
stdout listener must obey the same bounded behavior for both line framing and
`Content-Length` framing.

## Invariants

1. MCP child stdout line framing must not use allocation-unbounded
   `read_line`.
2. MCP child stdout header parsing must bound each header/log line before
   searching for `Content-Length`.
3. MCP child stdout body allocation must reject `Content-Length` values above
   the declared frame maximum before allocating the body buffer.
4. Oversized line or body frames are protocol failures for that upstream child,
   not recoverable protocol states.
5. The fix stays inside the EasyNet MCP product adapter. Axon Invocation
   signing, admission, receipt, and lifecycle semantics are unchanged.

## Effect

This slice preserves public behavior for valid MCP servers. Invalid or hostile
child stdout that exceeds the frame bound now fails before unbounded memory is
allocated.
