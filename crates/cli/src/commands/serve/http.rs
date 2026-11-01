//! Serving HTTP/1 on a listener, with the timeouts `axum::serve` does not
//! take: a request's head must arrive within the read timeout, every write
//! must go out within the write timeout, and a stop waits a grace period for
//! requests in flight before it closes what is left (SV-02, IN-11).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::{Instant, Sleep};

/// How long serve waits on the network.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timeouts {
    /// How long a request's head may take to arrive. A connection kept alive
    /// between requests is closed when the next one takes longer. The body
    /// of a write has as long again, which the write reads for itself.
    pub(crate) read: Duration,
    /// How long one write to a connection may wait to go out: a page that
    /// stops reading, such as an event stream's, is cut off after it.
    pub(crate) write: Duration,
    /// How long a stop waits for requests in flight before it closes their
    /// connections.
    pub(crate) grace: Duration,
}

impl Timeouts {
    /// The timeouts `cases serve` runs with.
    pub(crate) const SERVE: Timeouts = Timeouts {
        read: Duration::from_secs(30),
        write: Duration::from_secs(10),
        grace: Duration::from_secs(5),
    };
}

/// Serves `router` on `listener` until `stop` is done. Then `on_stop` runs,
/// to end requests that would not end by themselves, such as event streams;
/// the connections are asked to close once their request in flight is
/// answered, and those still open after the grace period are closed.
/// Gives how many were closed that way.
pub(crate) async fn serve(
    listener: TcpListener,
    router: Router,
    timeouts: Timeouts,
    stop: impl Future<Output = ()>,
    on_stop: impl FnOnce(),
) -> usize {
    let graceful = GracefulShutdown::new();
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(timeouts.read);
    let mut connections = JoinSet::new();
    let mut stop = std::pin::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => break,
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) => {
                        pause_after(&error).await;
                        continue;
                    }
                };
                let io = TokioIo::new(WriteTimeout::new(stream, timeouts.write));
                let service = TowerToHyperService::new(router.clone());
                let connection = graceful.watch(builder.serve_connection(io, service));
                connections.spawn(async move {
                    // A connection that fails, as when the page goes away,
                    // has no one to tell.
                    let _ = connection.await;
                });
                // Reap the connections that are done, so the set holds only
                // those still open.
                while connections.try_join_next().is_some() {}
            }
        }
    }
    drop(listener);
    on_stop();
    let finished = tokio::time::timeout(timeouts.grace, graceful.shutdown()).await;
    // A task stays in the set, and in its count, until it is reaped: reap
    // the connections that ended since the last accept, so that only those
    // still open are counted.
    while connections.try_join_next().is_some() {}
    let left = if finished.is_ok() {
        0
    } else {
        connections.len()
    };
    // Dropping the set aborts every connection task still running, which
    // closes its connection.
    drop(connections);
    left
}

/// Waits after a failed accept. A connection the peer gave up on before it
/// was accepted costs nothing; anything else, such as running out of file
/// descriptors, is given a second before the next try, as `axum::serve`
/// does.
async fn pause_after(error: &io::Error) {
    if !matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
    ) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// A connection whose writes fail when they cannot go out within a time: a
/// page that stops reading fills the connection's buffers, and a write then
/// waits where neither the page going away nor a stop can reach it.
#[derive(Debug)]
pub(crate) struct WriteTimeout<S> {
    inner: S,
    timeout: Duration,
    /// When the write waiting now gives up, while one waits.
    deadline: Option<Pin<Box<Sleep>>>,
}

impl<S> WriteTimeout<S> {
    pub(crate) fn new(inner: S, timeout: Duration) -> WriteTimeout<S> {
        WriteTimeout {
            inner,
            timeout,
            deadline: None,
        }
    }

    /// What a write that is `Pending` gives: still `Pending` until the
    /// timeout, then an error. A write that is done clears the deadline.
    fn guard<T>(&mut self, cx: &mut Context<'_>, poll: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        if poll.is_ready() {
            self.deadline = None;
            return poll;
        }
        let timeout = self.timeout;
        let deadline = self
            .deadline
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(Instant::now() + timeout)));
        if deadline.as_mut().poll(cx).is_ready() {
            self.deadline = None;
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a write did not go out in time",
            )));
        }
        Poll::Pending
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for WriteTimeout<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for WriteTimeout<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.guard(cx, poll)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.guard(cx, poll)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_flush(cx);
        self.guard(cx, poll)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.guard(cx, poll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that never takes a byte, as a socket whose buffers are full.
    struct Stuck;

    impl AsyncWrite for Stuck {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_write_that_cannot_go_out_fails_at_the_timeout() {
        use tokio::io::AsyncWriteExt;
        let mut stuck = WriteTimeout::new(Stuck, Duration::from_secs(10));
        let started = Instant::now();
        let error = stuck.write_all(b"id: v\n\n").await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(started.elapsed(), Duration::from_secs(10));
        let error = stuck.flush().await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    /// A writer whose every write waits once before it goes out, as a
    /// socket whose buffers are full for a moment.
    #[derive(Default)]
    struct Slow {
        waited: bool,
        taken: Vec<u8>,
    }

    impl AsyncWrite for Slow {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            if !self.waited {
                self.waited = true;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.waited = false;
            self.taken.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// A write that waited and then went out clears its deadline: a later
    /// write that waits is given the whole timeout again, not failed by the
    /// deadline the first one set.
    #[tokio::test(start_paused = true)]
    async fn a_write_that_goes_out_clears_the_deadline() {
        use tokio::io::AsyncWriteExt;
        let mut slow = WriteTimeout::new(Slow::default(), Duration::from_secs(10));
        slow.write_all(b"one").await.unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        slow.write_all(b" two").await.unwrap();
        assert_eq!(slow.inner.taken, b"one two");
    }
}
