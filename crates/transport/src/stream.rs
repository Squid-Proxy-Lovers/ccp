// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::{
    WRITE_CHUNK,
    websocket::{
        WebSocketStream,
        tungstenite::{Bytes, Message},
    },
};
use futures_util::{Sink, Stream};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{Duration, Instant},
};

/// A byte stream with one bounded incoming message and bounded parser write buffers.
/// Delivered Ping/Pong events and completed empty binary messages share a
/// 128-per-second limit. The WebSocket parser consumes non-final fragments;
/// those fragments are not counted by this message-level budget.
/// Tungstenite processes automatic Pong/Close writes while continuing to read when
/// writes would block; application reads must never wait for an outgoing flush.
pub struct ByteStream<S> {
    socket: WebSocketStream<S>,
    pending: Bytes,
    window: Instant,
    controls: u16,
    closing: bool,
    ended: bool,
}
impl<S> ByteStream<S> {
    /// Wrap a completed WebSocket configured with [`crate::websocket_config`].
    pub fn new(socket: WebSocketStream<S>) -> Self {
        Self {
            socket,
            pending: Bytes::new(),
            window: Instant::now(),
            controls: 0,
            closing: false,
            ended: false,
        }
    }
    fn control(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.controls = 0;
        }
        self.controls = self.controls.saturating_add(1);
        if self.controls > 128 {
            return Err(io::Error::other("carrier control-frame rate exceeded"));
        }
        Ok(())
    }
}
fn error(value: impl std::fmt::Display) -> io::Error {
    io::Error::other(value.to_string())
}
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for ByteStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if out.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        for _ in 0..32 {
            if !this.pending.is_empty() {
                let count = out.remaining().min(this.pending.len());
                out.put_slice(&this.pending.split_to(count));
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                return Poll::Ready(Ok(()));
            }
            if this.closing {
                std::task::ready!(Pin::new(&mut this.socket).poll_close(cx)).map_err(error)?;
                this.ended = true;
                return Poll::Ready(Ok(()));
            }
            match std::task::ready!(Pin::new(&mut this.socket).poll_next(cx)) {
                Some(Ok(Message::Binary(bytes))) if !bytes.is_empty() => this.pending = bytes,
                Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => {
                    this.control()?
                }
                Some(Ok(Message::Close(_))) => this.closing = true,
                None => this.ended = true,
                Some(Ok(_)) => {
                    return Poll::Ready(Err(io::Error::other("carrier requires binary frames")));
                }
                Some(Err(e)) => return Poll::Ready(Err(error(e))),
            }
        }
        // Cooperate between delivered empty messages and Ping/Pong events.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for ByteStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let this = self.get_mut();
        std::task::ready!(Pin::new(&mut this.socket).poll_ready(cx)).map_err(error)?;
        let count = bytes.len().min(WRITE_CHUNK);
        let chunk = bytes
            .get(..count)
            .ok_or_else(|| io::Error::other("invalid carrier write length"))?;
        Pin::new(&mut this.socket)
            .start_send(Message::Binary(Bytes::copy_from_slice(chunk)))
            .map_err(error)?;
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket)
            .poll_flush(cx)
            .map_err(error)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket)
            .poll_close(cx)
            .map_err(error)
    }
}
