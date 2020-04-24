use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::ready;
use tokio::io::{AsyncRead, AsyncWrite};

/// This struct is created by calling [`transfer_uni`][transfer_uni].
///
/// [transfer_uni]: transfer_uni()
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct TransferUni<R, W> {
    reader: R,
    read_done: bool,
    writer: W,
    pos: usize,
    cap: usize,
    amt: u64,
    buf: Box<[u8]>,
}

/// This is a copy/paste of the [`tokio::io::copy`][tokio] modified to
/// own the reader and writer.
/// 
/// Additionally this function propagates EOF from `reader` to `writer`
/// by calling a `poll_shutdown` on the latter when EOF is received.
///
/// [tokio]: tokio::io::copy
pub fn transfer_uni<R, W>(reader: R, writer: W) -> TransferUni<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    TransferUni {
        reader,
        read_done: false,
        writer,
        amt: 0,
        pos: 0,
        cap: 0,
        buf: Box::new([0; 2048]),
    }
}

impl<R, W> TransferUni<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// Consumes this transfer, returning the underlying reader and writer.
    pub fn into_inner(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

impl<R, W> Future for TransferUni<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    type Output = io::Result<u64>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        loop {
            // If our buffer is empty, then we need to read some data to
            // continue.
            if self.pos == self.cap && !self.read_done {
                let me = &mut *self;
                let n = ready!(Pin::new(&mut me.reader).poll_read(cx, &mut me.buf))?;
                if n == 0 {
                    self.read_done = true;
                } else {
                    self.pos = 0;
                    self.cap = n;
                }
            }

            // If our buffer has some data, let's write it out!
            while self.pos < self.cap {
                let me = &mut *self;
                let i = ready!(Pin::new(&mut me.writer).poll_write(cx, &me.buf[me.pos..me.cap]))?;
                if i == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write zero byte into writer",
                    )));
                } else {
                    self.pos += i;
                    self.amt += i as u64;
                }
            }

            // If we've written all the data and we've seen EOF, flush out the
            // data and finish the transfer.
            if self.pos == self.cap && self.read_done {
                let me = &mut *self;
                ready!(Pin::new(&mut me.writer).poll_flush(cx))?;
                ready!(Pin::new(&mut me.writer).poll_shutdown(cx))?;
                return Poll::Ready(Ok(self.amt));
            }
        }
    }
}
