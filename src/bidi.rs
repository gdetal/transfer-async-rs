use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{split, AsyncRead, AsyncWrite, ReadHalf, WriteHalf};

use pin_project_lite::pin_project;

use crate::{transfer_uni, TransferUni};

pin_project! {
    /// A future that asynchronously bidirectionnaly copies the entire contents
    /// of a stream into another stream.
    ///
    /// This struct is generally created by calling [`transfer`][transfer].
    /// Please see the documentation of `transfer()` for more details.
    ///
    /// [transfer]: transfer()
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct Transfer<I, O> {
        #[pin]
        t0: TransferUni<ReadHalf<I>, WriteHalf<O>>,
        #[pin]
        t1: TransferUni<ReadHalf<O>, WriteHalf<I>>,
        t0_done: bool,
        t1_done: bool,
        t0_amt: u64,
        t1_amt: u64,
    }
}

/// Asynchronously bidirectionnaly copies the entire contents of a stream into
/// another stream. A stream must implement [`tokio::io::AsyncRead`][tokio] and
/// [`tokio::io::AsyncWrite`][tokio].
///
/// This function returns a future that will continuously read data from
/// `inbound` and then write it into `outbound` (and vice-versa) in a streaming
/// fashion until `reader` returns EOF.
///
/// On success, the total number of bytes that were copied from `inbound` to
/// `outbound` is returned and vice-versa.
///
/// [tokio]: tokio::io::AsyncRead
/// [tokio]: tokio::io::AsyncWrite
///
/// # Errors
///
/// The returned future will finish with an error will return an error
/// immediately if any call to `poll_read` or `poll_write` returns an error.
///
/// # Examples
///
/// ```
/// use transfer_async;
///
/// # async fn dox() -> std::io::Result<()> {
/// 
/// # Ok(())
/// # }
/// ```
pub fn transfer<I, O>(inbound: I, outbound: O) -> Transfer<I, O>
where
    I: AsyncRead + AsyncWrite,
    O: AsyncRead + AsyncWrite,
{
    let (ri, wi) = split(inbound);
    let (ro, wo) = split(outbound);

    Transfer {
        t0: transfer_uni(ri, wo),
        t1: transfer_uni(ro, wi),
        t0_done: false,
        t1_done: false,
        t0_amt: 0,
        t1_amt: 0,
    }
}

impl<I, O> Transfer<I, O>
where
    I: AsyncRead + AsyncWrite,
    O: AsyncRead + AsyncWrite,
{
    pub fn into_inner(self) -> (I, O) {
        let (ri, wo) = self.t0.into_inner();
        let (ro, wi) = self.t1.into_inner();
        (ri.unsplit(wi), ro.unsplit(wo))
    }
}

impl<I, O> Future for Transfer<I, O>
where
    I: AsyncRead + AsyncWrite,
    O: AsyncRead + AsyncWrite,
{
    type Output = io::Result<(u64, u64)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if !*this.t0_done {
            if let Poll::Ready(amt) = this.t0.poll(cx) {
                *this.t0_amt = amt?;
                *this.t0_done = true;
            }
        }

        if !*this.t1_done {
            if let Poll::Ready(amt) = this.t1.poll(cx) {
                *this.t1_amt = amt?;
                *this.t1_done = true;
            }
        }

        if *this.t0_done && *this.t1_done {
            Poll::Ready(Ok((*this.t0_amt, *this.t1_amt)))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::io::{AsyncWriteExt, AsyncReadExt};

    #[tokio::test]
    async fn test_tcp_proxy() {
        let proxy = TcpListener::bind("127.0.0.1:12443").await.unwrap();
        let server = TcpListener::bind("127.0.0.1:12442").await.unwrap();

        let task_proxy = tokio::spawn(proxy_loop(proxy));
        let task_server = tokio::spawn(server_loop(server));

        // create the client connection (via proxy)
        let mut client = TcpStream::connect("127.0.0.1:12443").await.unwrap();

        client.write(b"test").await.unwrap();
        drop(client);
        
        task_proxy.await.unwrap();
        task_server.await.unwrap();
    }

    async fn proxy_loop(mut proxy: tokio::net::TcpListener) {
        // expect a single connection:
        let (socket, _) = proxy.accept().await.unwrap();
        // perform a connect
        let other = TcpStream::connect("127.0.0.1:12442").await.unwrap();

        let _res = transfer(socket, other).await.unwrap();
        // TODO validate res
    }

    async fn server_loop(mut server: tokio::net::TcpListener) {
        // expect a single connection:
        let (mut socket, _) = server.accept().await.unwrap();
        // echo back to the client
        loop {
            let mut buffer = [0; 4096];

            socket.read(&mut buffer).await.unwrap();
            socket.write(&buffer).await.unwrap();
        }
    }
}
