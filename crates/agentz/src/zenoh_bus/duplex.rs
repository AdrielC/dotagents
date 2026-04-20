//! Byte-oriented duplex backed by a Zenoh subscriber (read side) and a Zenoh publisher (write side).
//!
//! The duplex is `AsyncRead` + `AsyncWrite`, so you can drop it into any protocol implementation
//! that consumes generic async I/O — [`rmcp`](https://docs.rs/rmcp) for MCP, an [`agent-client-protocol`]
//! JSON-RPC framing layer, or a plain newline-delimited line transport.
//!
//! # Flow control
//!
//! - **Writes** are queued into a bounded channel drained by a single background task that awaits
//!   `Session::put` in order. This preserves write ordering, which JSON-RPC framing requires — a
//!   naive `tokio::spawn` per write can reorder samples on the wire.
//! - **Reads** pull from an mpsc channel fed by a background Tokio task attached to the
//!   subscriber. Each Zenoh `Sample` pushes its payload as one chunk.

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
    pending: Vec<u8>,
    pos: usize,
}

impl AsyncRead for ZenohDuplexReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.pos < self.pending.len() {
                let remaining = &self.pending[self.pos..];
                let n = remaining.len().min(buf.remaining());
                buf.put_slice(&remaining[..n]);
                self.pos += n;
                if self.pos == self.pending.len() {
                    self.pending.clear();
                    self.pos = 0;
                }
                return Poll::Ready(Ok(()));
            }
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    self.pending = chunk;
                    self.pos = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Write half of a Zenoh-backed duplex. Writes are pushed into a serialized background publisher
/// so samples reach the outbound key in the order `poll_write` accepted them.
pub struct ZenohDuplexWriter {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl AsyncWrite for ZenohDuplexWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.tx.send(buf.to_vec()).is_err() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "zenoh duplex writer task has exited",
            )));
        }
        Poll::Ready(Ok(buf.len()))
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
        let subscriber = session.declare_subscriber(inbound_key.as_str()).await?;
        let (rx_tx, rx) = mpsc::channel::<Vec<u8>>(256);
        tokio::spawn(async move {
            let mut stream = subscriber.stream();
            while let Some(sample) = stream.next().await {
                let bytes: Vec<u8> = sample.payload().to_bytes().into_owned();
                if rx_tx.send(bytes).await.is_err() {
                    break;
                }
            }
        });

        let (tx, mut tx_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer_session = session.clone();
        let writer_key: Arc<str> = Arc::from(outbound_key);
        tokio::spawn(async move {
            while let Some(bytes) = tx_rx.recv().await {
                if writer_session
                    .put(writer_key.as_ref(), bytes)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        Ok(ZenohDuplex {
            reader: ZenohDuplexReader {
                rx,
                pending: Vec::new(),
                pos: 0,
            },
            writer: ZenohDuplexWriter { tx },
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
