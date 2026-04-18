//! Byte-oriented duplex backed by a Zenoh subscriber (read side) and a Zenoh publisher (write side).
//!
//! The duplex is `AsyncRead` + `AsyncWrite`, so you can drop it into any protocol implementation
//! that consumes generic async I/O — [`rmcp`](https://docs.rs/rmcp) for MCP, an [`agent-client-protocol`]
//! JSON-RPC framing layer, or a plain newline-delimited line transport.
//!
//! # Flow control
//!
//! - **Writes** are non-blocking: bytes are published immediately on the outbound key.
//! - **Reads** pull from a `mpsc::channel` fed by a background Tokio task attached to the
//!   subscriber. Each Zenoh `Sample` pushes its payload as one chunk.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;
use zenoh::Session;

/// Read half of a Zenoh-backed duplex.
pub struct ZenohDuplexReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

impl AsyncRead for ZenohDuplexReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if !self.buf.is_empty() {
                let to_copy = std::cmp::min(buf.remaining(), self.buf.len());
                for _ in 0..to_copy {
                    if let Some(b) = self.buf.pop_front() {
                        buf.put_slice(&[b]);
                    }
                }
                return Poll::Ready(Ok(()));
            }
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    self.buf.extend(chunk);
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Write half of a Zenoh-backed duplex. Each `poll_write` is a Zenoh `put` on the outbound key.
pub struct ZenohDuplexWriter {
    session: Arc<Session>,
    key: Arc<str>,
}

impl AsyncWrite for ZenohDuplexWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let session = self.session.clone();
        let key = self.key.clone();
        let payload = buf.to_vec();
        let len = payload.len();
        // zenoh `put().await` can be backgrounded; we don't need the handle.
        tokio::spawn(async move {
            let _ = session.put(key.as_ref(), payload).await;
        });
        Poll::Ready(Ok(len))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Split duplex. Produced by [`super::ZenohBus::duplex`].
pub struct ZenohDuplex {
    reader: ZenohDuplexReader,
    writer: ZenohDuplexWriter,
}

impl ZenohDuplex {
    pub async fn open(
        session: Arc<Session>,
        inbound_key: String,
        outbound_key: String,
    ) -> zenoh::Result<Self> {
        let subscriber = session
            .declare_subscriber(inbound_key.as_str())
            .await?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
        tokio::spawn(async move {
            let mut stream = subscriber.stream();
            while let Some(sample) = stream.next().await {
                let bytes: Vec<u8> = sample.payload().to_bytes().into_owned();
                if tx.send(bytes).await.is_err() {
                    break;
                }
            }
        });
        Ok(ZenohDuplex {
            reader: ZenohDuplexReader { rx, buf: VecDeque::new() },
            writer: ZenohDuplexWriter {
                session,
                key: Arc::<str>::from(outbound_key),
            },
        })
    }

    pub fn split(self) -> (ZenohDuplexReader, ZenohDuplexWriter) {
        (self.reader, self.writer)
    }
}

impl AsyncRead for ZenohDuplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for ZenohDuplex {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}
