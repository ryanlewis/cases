//! `GET /v1/changes`: the event stream that tells a page the store moved
//! (IN-06).
//!
//! Each message is the store's version, as its id and its data. The stream
//! sends the version at once unless the page has it already, as `?after=V`
//! or the `Last-Event-ID` a browser sends when it reconnects, and then each
//! time a read of the store finds it moved, with at least [`GAP`] between
//! two messages, and the version as it is when the message goes, so that no
//! change goes unsent. The page reads what it shows from the API on each.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::header::{self, HeaderMap};
use axum::response::{IntoResponse, Response};
use http_body::Frame;
use tokio::sync::mpsc;

use super::app::{self, App};

/// The least time between two messages on one stream: as often as serve
/// reads the store by itself. A message makes the page read the API, and
/// during a burst of writes, such as a sweep, one refresh a second is
/// enough.
pub(crate) const GAP: Duration = Duration::from_secs(1);

/// Opens a page's event stream.
pub(crate) async fn changes(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let mut after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if after.is_none() {
        after = form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .find(|(name, _)| name == "after")
            .map(|(_, value)| value.into_owned());
    }
    // Bring the version up to the store first. A store that cannot be read
    // leaves it as it was; the page's own reads report the error.
    let _ = app::blocking(&app, |app| Ok(app.watch.read())).await;
    let (sender, receiver) = mpsc::channel(1);
    tokio::spawn(send(app, after.unwrap_or_default(), sender));
    // The headers go out even when the stream ends at once, as when serve
    // is stopping, so that the browser tries again rather than give up.
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::new(Messages(receiver)),
    )
        .into_response()
}

/// Sends the stream's messages until the page goes away or serve stops.
async fn send(app: Arc<App>, mut after: String, sender: mpsc::Sender<Bytes>) {
    let mut snapshots = app.watch.subscribe();
    let mut stopping = app.stopping.clone();
    let stopped = |stopping: &mut tokio::sync::watch::Receiver<bool>| *stopping.borrow_and_update();
    if stopped(&mut stopping) {
        return;
    }
    loop {
        let version = snapshots.borrow_and_update().version.clone();
        if version != after {
            let message = format!("id: {version}\ndata: {version}\n\n");
            if sender.send(Bytes::from(message)).await.is_err() {
                return;
            }
            after = version;
            tokio::select! {
                () = tokio::time::sleep(GAP) => continue,
                () = sender.closed() => return,
                _ = stopping.changed() => return,
            }
        }
        tokio::select! {
            changed = snapshots.changed() => if changed.is_err() { return },
            () = sender.closed() => return,
            _ = stopping.changed() => return,
        }
    }
}

/// The body of an event stream: the messages [`send`] gives, until it ends.
#[derive(Debug)]
struct Messages(mpsc::Receiver<Bytes>);

impl http_body::Body for Messages {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        self.0
            .poll_recv(cx)
            .map(|message| message.map(|message| Ok(Frame::data(message))))
    }
}
