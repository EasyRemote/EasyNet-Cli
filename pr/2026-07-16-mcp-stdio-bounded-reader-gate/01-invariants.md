# Invariants: MCP stdio bounded reader

## Bounded allocation

The daemon MCP stdio transport must decide whether a line or frame is oversized
before retaining bytes beyond the configured limit.

## Single owner

The MCP stdio owner is responsible for decoding child stdout and server stdin.
Callers must not hand-roll line or frame reads around it.

## Content-Length handling

`Content-Length` is metadata, not permission to allocate. The value must be
parsed, compared with `MAX_CHILD_STDIO_FRAME_BYTES`, and rejected before body
allocation or `read_exact`.

## Failure behavior

Oversized input is a protocol error. The reader may drain to the next frame
boundary, but it must not keep the oversized payload in memory.
