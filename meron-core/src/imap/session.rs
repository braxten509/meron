//! Transport state, reads, writes and IDLE.
//!
//! Tracks read progress and negotiated capabilities for a single connection.
//! Supports IDLE watchers and checks whether pooled connections can be reused.

use super::*;

/// Connection stream: implicit TLS (port 993 etc.) or plaintext (e.g. a local
/// test server on 3143). One enum so the async-imap `Session` type is uniform.
/// STARTTLS upgrade is not yet supported.
#[derive(Debug)]
pub enum Stream {
    Plain(TcpStream, ConnState),
    Tls(Box<TlsStream<TcpStream>>, ConnState),
}

pub(crate) type ReadProgress = std::sync::Arc<std::sync::Mutex<Option<tokio::time::Instant>>>;

/// What is kept beside one connection's socket for as long as it lives.
#[derive(Default)]
pub struct ConnState {
    progress: ReadProgress,
    /// The server's capability list, once it has been asked for (see
    /// [`capabilities_once`]).
    capabilities: Option<std::sync::Arc<async_imap::types::Capabilities>>,
}

impl std::fmt::Debug for ConnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnState")
            .field("capabilities_known", &self.capabilities.is_some())
            .finish_non_exhaustive()
    }
}

impl Stream {
    pub fn plain(socket: TcpStream) -> Self {
        Self::Plain(socket, Default::default())
    }

    pub fn tls(socket: Box<TlsStream<TcpStream>>) -> Self {
        Self::Tls(socket, Default::default())
    }

    pub(crate) fn observe_reads(&mut self, progress: ReadProgress) {
        self.state_mut().progress = progress;
    }

    fn state(&self) -> &ConnState {
        match self {
            Self::Plain(_, state) | Self::Tls(_, state) => state,
        }
    }

    fn state_mut(&mut self) -> &mut ConnState {
        match self {
            Self::Plain(_, state) | Self::Tls(_, state) => state,
        }
    }
}

/// The server's capability list, asked for once per connection. A session is
/// only ever handed out signed in, and a server does not change what it
/// advertises after that, so every later check on the same connection reuses
/// the first answer instead of spending a round trip on CAPABILITY (a delete
/// used to ask five times). A failed ask is not remembered.
pub(crate) async fn capabilities_once(
    session: &mut Session,
) -> async_imap::error::Result<std::sync::Arc<async_imap::types::Capabilities>> {
    if let Some(known) = &session.get_ref().state().capabilities {
        return Ok(known.clone());
    }
    let asked = std::sync::Arc::new(session.capabilities().await?);
    session.get_mut().state_mut().capabilities = Some(asked.clone());
    Ok(asked)
}

/// How long an IDLE watcher keeps listening after a removal or flag notice
/// before it re-reads the folder (see [`settle_idle`]).
pub const IDLE_SETTLE_QUIET: std::time::Duration = std::time::Duration::from_millis(300);

/// Longest [`settle_idle`] holds a re-read back, however long the burst runs.
pub const IDLE_SETTLE_CAP: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether an IDLE notice announces new mail (EXISTS or RECENT) rather than
/// a removal or a flag change.
fn idle_notice_is_new_mail(response: &async_imap::extensions::idle::IdleResponse) -> bool {
    use async_imap::imap_proto::{MailboxDatum, Response};
    match response {
        async_imap::extensions::idle::IdleResponse::NewData(data) => matches!(
            data.parsed(),
            Response::MailboxData(MailboxDatum::Exists(_) | MailboxDatum::Recent(_))
        ),
        _ => false,
    }
}

/// After the IDLE notice `first`, keep listening on the same IDLE until the
/// server has said nothing more for `quiet`, or `cap` has passed in all.
///
/// Removals and flag changes come in bursts: a bulk delete here or on another
/// device is one notice per message, and re-reading the folder after each of
/// them is all wasted but the last. One re-read after the burst sees the same
/// result. New mail is not held back: this returns at once when `first`, or
/// any notice during the wait, announces it.
pub async fn settle_idle(
    handle: &mut async_imap::extensions::idle::Handle<Stream>,
    first: &async_imap::extensions::idle::IdleResponse,
    quiet: std::time::Duration,
    cap: std::time::Duration,
) -> async_imap::error::Result<()> {
    use async_imap::extensions::idle::IdleResponse;
    if !matches!(first, IdleResponse::NewData(_)) || idle_notice_is_new_mail(first) {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + cap;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        let (wait, _stop) = handle.wait_with_timeout(quiet.min(left));
        let next = wait.await?;
        if !matches!(next, IdleResponse::NewData(_)) || idle_notice_is_new_mail(&next) {
            return Ok(());
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let (result, state) = match self.get_mut() {
            Stream::Plain(s, state) => (Pin::new(s).poll_read(cx, buf), state),
            Stream::Tls(s, state) => (Pin::new(s.as_mut()).poll_read(cx, buf), state),
        };
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            let mut clock = state.progress.lock().unwrap();
            if clock.is_some() {
                *clock = Some(tokio::time::Instant::now());
            }
        }
        result
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s, _) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s, _) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s, _) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s, _) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s, _) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s, _) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

pub type Session = async_imap::Session<Stream>;

/// Whether an idle session's socket still looks open. A server that drops an
/// idle connection leaves its FIN (often behind a BYE or TLS close_notify)
/// queued on the socket, and the runtime's reactor records that as read-closed
/// readiness, so a dead pooled session can be spotted without a round trip.
/// This cannot see a close that lands after the check; callers still need to
/// handle a command failing on a dropped connection.
pub fn session_looks_open(session: &Session) -> bool {
    match session.get_ref() {
        Stream::Plain(stream, _) => tcp_looks_open(stream),
        Stream::Tls(stream, _) => tcp_looks_open(stream.get_ref().0),
    }
}

pub(super) fn tcp_looks_open(tcp: &TcpStream) -> bool {
    use futures::FutureExt;
    use tokio::io::Interest;

    match tcp.ready(Interest::READABLE).now_or_never() {
        Some(Ok(ready)) => !(ready.is_read_closed() || ready.is_error()),
        Some(Err(_)) => false,
        // Nothing pending on the socket: the connection is quiet, not closed.
        None => true,
    }
}
