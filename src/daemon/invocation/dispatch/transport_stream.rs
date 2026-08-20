//! Transport stream lifecycle helpers.
//!
//! The daemon Invocation transport must surface a dropped gRPC response stream
//! as an explicit runtime lifecycle signal. Axon `LocalRuntime` deliberately
//! does not make handle drop mutate invocation state, so daemon-owned adapters
//! wrap their down streams here and drive cancellation/close from the transport
//! boundary that actually observes the client disconnect.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tonic::Status;

/// Down-stream wrapper that reports response-stream drop to the owner task.
///
/// The notification is best-effort and non-blocking: if the owner has already
/// completed and dropped the receiver, stream drop is terminally irrelevant.
pub(crate) struct TransportDropNotifyStream<S> {
    inner: Pin<Box<S>>,
    close_tx: Option<tokio::sync::mpsc::Sender<String>>,
    reason: &'static str,
}

impl<S> TransportDropNotifyStream<S>
where
    S: Stream + Send + 'static,
{
    pub(crate) fn new(
        inner: S,
        close_tx: tokio::sync::mpsc::Sender<String>,
        reason: &'static str,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            close_tx: Some(close_tx),
            reason,
        }
    }
}

impl<S, T> Stream for TransportDropNotifyStream<S>
where
    S: Stream<Item = Result<T, Status>> + Send + 'static,
{
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl<S> Drop for TransportDropNotifyStream<S> {
    fn drop(&mut self) {
        if let Some(close_tx) = self.close_tx.take() {
            let _ = close_tx.try_send(self.reason.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;

    use super::*;

    #[tokio::test]
    async fn transport_drop_notify_stream_signals_response_drop() {
        let (_down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<(), Status>>(1);
        let (close_tx, mut close_rx) = tokio::sync::mpsc::channel::<String>(1);
        let stream = TransportDropNotifyStream::new(
            tokio_stream::wrappers::ReceiverStream::new(down_rx),
            close_tx,
            "test response stream dropped",
        );

        drop(stream);

        let reason = tokio::time::timeout(std::time::Duration::from_secs(1), close_rx.recv())
            .await
            .expect("drop notification should not block")
            .expect("drop notification should be sent");
        assert_eq!(reason, "test response stream dropped");
    }

    #[tokio::test]
    async fn transport_drop_notify_stream_forwards_inner_items() {
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<u8, Status>>(1);
        let (close_tx, _close_rx) = tokio::sync::mpsc::channel::<String>(1);
        down_tx.send(Ok(7)).await.expect("send test frame");
        drop(down_tx);
        let mut stream = TransportDropNotifyStream::new(
            tokio_stream::wrappers::ReceiverStream::new(down_rx),
            close_tx,
            "test response stream dropped",
        );

        assert_eq!(stream.next().await.expect("first item").expect("ok"), 7);
        assert!(stream.next().await.is_none());
    }
}
