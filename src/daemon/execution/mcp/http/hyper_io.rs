// EasyNet CLI — Streamable HTTP / hyper ↔ tokio IO adapter
// ========================================================
//
// File: src/daemon/execution/mcp_client/http/hyper_io.rs
//
// Minimal hyper IO adapter for tokio `AsyncRead` / `AsyncWrite`.
// `hyper` 1.x expects its own `hyper::rt::{Read, Write}` traits;
// tokio's IO surfaces need a thin shim. This is what
// `hyper-util`'s `TokioIo` provides; we inline a minimal version
// here to avoid pulling `hyper-util` into the non-`axon-pb` build
// path (it's only used here, and pulls a noticeable dep tree).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(super) struct HyperTokioIo<T> {
    inner: T,
}

impl<T> HyperTokioIo<T> {
    pub(super) fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: tokio::io::AsyncRead + Unpin> hyper::rt::Read for HyperTokioIo<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // SAFETY: the unfilled portion of the cursor is mut-borrowed
        // and we only write into it via tokio's ReadBuf, which only
        // writes initialised bytes — we then call advance() with
        // exactly the count tokio reports as filled. Standard
        // pattern from hyper-util::rt::TokioIo.
        let n = unsafe {
            let mut tbuf = tokio::io::ReadBuf::uninit(buf.as_mut());
            match std::pin::Pin::new(&mut self.inner).poll_read(cx, &mut tbuf) {
                std::task::Poll::Ready(Ok(())) => tbuf.filled().len(),
                other => return other,
            }
        };
        unsafe {
            buf.advance(n);
        }
        std::task::Poll::Ready(Ok(()))
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> hyper::rt::Write for HyperTokioIo<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
