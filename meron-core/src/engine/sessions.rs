//! Manages the pool of authenticated IMAP connections.
//! Reads, writes and transfers choose a session with the appropriate retry policy.
//! The engine returns healthy sessions to the pool after each operation.

use super::*;

impl Engine {
    /// Pop the hottest non-expired pooled session for `account`, discarding any
    /// that have been idle past `MAX_IDLE` or whose socket the server has
    /// already closed.
    pub fn take_pooled(&self, account: &str) -> Option<imap::Session> {
        loop {
            let session = pool_take(
                &mut self.pool.lock().unwrap(),
                account,
                std::time::Instant::now(),
                MAX_IDLE,
            )?;
            if imap::session_looks_open(&session) {
                return Some(session);
            }
            pool_debug(account, "closed-evict");
        }
    }

    /// Return a healthy session to the pool, or drop it if the account is paused or already at `MAX_POOLED` (the transient-overflow case).
    ///
    /// The paused check must happen here, not only when `account.setPaused` clears the pool: a
    /// background sync can already have a session checked out when the account is paused and otherwise
    /// return that old session after the clear.
    pub fn return_pooled(&self, account: &str, session: imap::Session) {
        if self.is_paused(account) {
            return;
        }
        pool_return(
            &mut self.pool.lock().unwrap(),
            account,
            session,
            std::time::Instant::now(),
            MAX_POOLED,
        );
    }

    /// Drop every pooled session for `account`. Called when credentials become
    /// invalid (account removed) or checking is paused, so we never reuse a
    /// session built from stale creds.
    pub fn clear_pool(&self, account: &str) {
        self.pool.lock().unwrap().remove(account);
    }

    /// Drop every warm pooled session. Used on OS resume: a session's freshness
    /// is judged by elapsed `Instant`, but the monotonic clock is frozen across
    /// suspend, so a connection dead for hours still looks recently used and
    /// dodges the `MAX_IDLE` eviction. Clearing forces the next op to reconnect.
    pub fn clear_all_pools(&self) {
        self.pool.lock().unwrap().clear();
    }

    pub(super) fn connect_lock(&self, account: &str) -> Arc<Mutex<()>> {
        self.connect_locks
            .lock()
            .unwrap()
            .entry(account.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Run `f` against a warm pooled session when one is available, otherwise a freshly connected one.
    ///
    /// On success the session is returned to the pool.
    ///
    /// `retry`: when a *pooled* session fails (the likely cause being that the server silently dropped
    /// it), reconnect fresh and run `f` once more. Only safe for read-only operations — a stale pooled
    /// connection fails on its first command (before any mutation), but a connection that drops *after*
    /// a mutating command reached the server must not be retried.
    ///
    /// Use [`with_read_session`](Self::with_read_session) /
    /// [`with_write_session`](Self::with_write_session) instead of calling this directly.
    pub async fn with_session<T, F>(&self, account: &str, retry: bool, f: F) -> anyhow::Result<T>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        let usage = if retry {
            SessionUse::Read
        } else {
            SessionUse::Write
        };
        self.run_session(account, usage, f).await
    }

    async fn run_session<T, F>(
        &self,
        account: &str,
        usage: SessionUse,
        mut f: F,
    ) -> anyhow::Result<T>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        let mut command_attempted = false;
        if let Some(session) = self.take_pooled(account) {
            match self
                .run_pooled(account, usage, session, &mut f, &mut command_attempted)
                .await
            {
                Some(result) => return result,
                // Fall through to a fresh connection and try once more.
                None => pool_debug(account, "stale-retry"),
            }
        }

        pool_debug(account, "fresh-connect");
        let connect_lock = self.connect_lock(account);
        let mut connect_guard = tokio::time::timeout(
            match usage {
                SessionUse::Transfer { deadline, .. } => connect_coordination_timeout()
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                _ => connect_coordination_timeout(),
            },
            connect_lock.lock(),
        )
        .await
        .ok();
        if connect_guard.is_none() {
            crate::mlog!(
                crate::log::Level::Warn,
                "net",
                "fresh-connect coordination timed out for {account}; connecting independently"
            );
        }
        // The task ahead of us may have completed its operation and returned the
        // session while we waited for its handshake coordinator.
        if let Some(session) = self.take_pooled(account) {
            drop(connect_guard.take());
            match self
                .run_pooled(account, usage, session, &mut f, &mut command_attempted)
                .await
            {
                Some(result) => return result,
                None => {
                    pool_debug(account, "stale-retry");
                    connect_guard = tokio::time::timeout(
                        match usage {
                            SessionUse::Transfer { deadline, .. } => connect_coordination_timeout()
                                .min(
                                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                                ),
                            _ => connect_coordination_timeout(),
                        },
                        connect_lock.lock(),
                    )
                    .await
                    .ok();
                }
            }
        }
        // The caller's wait is bounded. ensure_valid_creds owns a spawned task,
        // so cancellation still lets a rotated token reach durable storage.
        let creds = match usage {
            SessionUse::Transfer { deadline, .. } => {
                tokio::time::timeout_at(deadline, self.ensure_valid_creds(account))
                    .await
                    .map_err(|_| anyhow::anyhow!("IMAP session preparation timed out"))??
            }
            _ => self.ensure_valid_creds(account).await?,
        };
        let connect_started = std::time::Instant::now();
        let mut session = match usage {
            // Not `TransferTimedOut`: no transfer started, and callers read
            // that one as the message being too slow to download.
            SessionUse::Transfer { deadline, .. } => {
                tokio::time::timeout_at(deadline, imap::connect(&creds))
                    .await
                    .map_err(|_| anyhow::anyhow!("IMAP connect timed out"))??
            }
            SessionUse::Write | SessionUse::Read => imap::connect(&creds).await?,
        };
        let connect_ms = connect_started.elapsed().as_millis();
        if connect_ms > 2_000 {
            crate::mlog!(
                crate::log::Level::Warn,
                "net",
                "slow IMAP connect for {account}: {connect_ms}ms"
            );
        }
        drop(connect_guard);
        let result = match usage {
            SessionUse::Transfer { deadline, budget } => {
                match tokio::time::timeout_at(deadline, f(&mut session)).await {
                    Ok(result) => result,
                    Err(_) => Err(TransferTimedOut(budget).into()),
                }
            }
            SessionUse::Write | SessionUse::Read => f(&mut session).await,
        };
        // A failed or timed-out command leaves the socket mid-command. Drop it
        // instead of returning it to the pool.
        let val = result?;
        self.return_pooled(account, session);
        Ok(val)
    }

    /// Run `f` on a pooled session. `None` means the session was stale (it has
    /// been dropped) and the caller should connect fresh and run `f` again.
    async fn run_pooled<T, F>(
        &self,
        account: &str,
        usage: SessionUse,
        mut session: imap::Session,
        f: &mut F,
        command_attempted: &mut bool,
    ) -> Option<anyhow::Result<T>>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        let may_retry = !*command_attempted;
        let result = match usage {
            // Writes must not be interrupted mid-command.
            SessionUse::Write => f(&mut session).await,
            // A bounded reuse attempt, so a dead pooled connection fails over
            // to a fresh one instead of hanging until TCP keepalive notices.
            SessionUse::Read => {
                // A second pooled checkout may also be stale after a network
                // change. Reject it before spending the command's only retry.
                if !may_retry {
                    let probe = tokio::time::timeout(
                        TRANSFER_PROBE_TIMEOUT,
                        session.run_command_and_check_ok("NOOP"),
                    )
                    .await;
                    if !matches!(probe, Ok(Ok(()))) {
                        return None;
                    }
                }
                *command_attempted = true;
                match tokio::time::timeout(POOLED_READ_TIMEOUT, f(&mut session)).await {
                    Ok(result) => result,
                    Err(_) if may_retry => return None,
                    Err(_) => return Some(Err(anyhow::anyhow!("pooled IMAP read timed out"))),
                }
            }
            // The short timeout covers a NOOP only: enough to notice a dead
            // socket without cutting a slow download short.
            SessionUse::Transfer { deadline, budget } => {
                let probe = tokio::time::timeout_at(
                    deadline,
                    tokio::time::timeout(
                        TRANSFER_PROBE_TIMEOUT,
                        session.run_command_and_check_ok("NOOP"),
                    ),
                )
                .await;
                if !matches!(probe, Ok(Ok(Ok(())))) {
                    return None;
                }
                *command_attempted = true;
                match tokio::time::timeout_at(deadline, f(&mut session)).await {
                    Ok(result) => result,
                    // Alive, only slow: starting over would not be faster.
                    Err(_) => return Some(Err(TransferTimedOut(budget).into())),
                }
            }
        };
        match result {
            Ok(val) => {
                pool_debug(account, "reuse");
                self.return_pooled(account, session);
                Some(Ok(val))
            }
            // Discard the suspect session (do not pool it).
            Err(error) => match usage {
                SessionUse::Write => Some(Err(error)),
                SessionUse::Read | SessionUse::Transfer { .. } if may_retry => None,
                // A session returned during coordination can serve the retry,
                // but its command failure must not trigger a third execution.
                SessionUse::Read | SessionUse::Transfer { .. } => Some(Err(error)),
            },
        }
    }

    /// Run a read-only operation against a pooled (or fresh) session, retrying
    /// once on a stale-connection failure.
    pub async fn with_read_session<T, F>(&self, account: &str, f: F) -> anyhow::Result<T>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        self.with_session(account, true, f).await
    }

    /// Run a read that transfers message bytes (a body FETCH).
    ///
    /// A reused socket is only given a short NOOP to prove it is alive. Wrapping the FETCH itself in
    /// [`POOLED_READ_TIMEOUT`] aborts a slow-but-alive transfer and starts it over, which is how a
    /// large or high-latency message ends up taking twice as long to appear.
    ///
    /// `budget` is for the whole call: the probe, the wait for a reconnect, the handshake, credential
    /// wait and a second run of `f` all come out of it. A spawned credential refresh continues if this
    /// caller times out. Running out mid-transfer fails with [`TransferTimedOut`]. `f` runs at most
    /// twice on pooled command errors.
    ///
    /// A session returned during coordination can serve the retry; otherwise a fresh connection does.
    pub async fn with_transfer_session<T, F>(
        &self,
        account: &str,
        budget: Duration,
        f: F,
    ) -> anyhow::Result<T>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        let deadline = tokio::time::Instant::now() + budget;
        self.run_session(account, SessionUse::Transfer { deadline, budget }, f)
            .await
    }

    /// Run a mutating operation against a pooled (or fresh) session. Never
    /// auto-retries, so a connection dropped mid-command can't double-apply.
    pub async fn with_write_session<T, F>(&self, account: &str, f: F) -> anyhow::Result<T>
    where
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        self.with_session(account, false, f).await
    }

    /// Run a mutating operation after a read-only preflight on the same session.
    ///
    /// If the preflight fails on a pooled session, discard that session and retry the preflight on a
    /// fresh connection. Once the mutating closure starts it is never retried.
    ///
    /// This is for commands such as folder deletion, which must first move a reused session away from
    /// the selected mailbox. Keeping the phases separate lets a dead pooled socket recover without
    /// risking a replay of a mutation whose server-side outcome is unknown.
    pub async fn with_preflighted_write_session<T, P, F>(
        &self,
        account: &str,
        preflight: P,
        f: F,
    ) -> anyhow::Result<T>
    where
        P: FnMut(&mut imap::Session) -> SessionOp<'_, ()> + Send,
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        self.preflighted_write(account, false, preflight, f).await
    }

    /// Like [`with_preflighted_write_session`](Self::with_preflighted_write_session), for idempotent
    /// writes such as `+FLAGS`/`-FLAGS` stores. The server can close a pooled connection between the
    /// preflight and the write, so when the write fails on a pooled session with a dropped connection
    /// it is run again, preflight included, on a fresh one. Applying it twice is harmless.
    pub async fn with_flag_write_session<T, P, F>(
        &self,
        account: &str,
        preflight: P,
        f: F,
    ) -> anyhow::Result<T>
    where
        P: FnMut(&mut imap::Session) -> SessionOp<'_, ()> + Send,
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        self.preflighted_write(account, true, preflight, f).await
    }

    async fn preflighted_write<T, P, F>(
        &self,
        account: &str,
        replay_dropped: bool,
        mut preflight: P,
        mut f: F,
    ) -> anyhow::Result<T>
    where
        P: FnMut(&mut imap::Session) -> SessionOp<'_, ()> + Send,
        F: FnMut(&mut imap::Session) -> SessionOp<'_, T> + Send,
        T: Send,
    {
        let replay = |operation_error: &anyhow::Error| {
            let dropped =
                replay_dropped && background_sync::is_transient_sync_error(operation_error);
            if dropped {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "net",
                    "pooled connection dropped during write for {account}, replaying on a fresh one: {operation_error:#}"
                );
            }
            dropped
        };
        if let Some(mut session) = self.take_pooled(account) {
            let ready =
                match tokio::time::timeout(POOLED_READ_TIMEOUT, preflight(&mut session)).await {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "pooled session preflight timed out after {}s",
                        POOLED_READ_TIMEOUT.as_secs()
                    )),
                };
            match ready {
                Ok(()) => {
                    let result = f(&mut session).await;
                    match result {
                        Ok(val) => {
                            pool_debug(account, "reuse");
                            self.return_pooled(account, session);
                            return Ok(val);
                        }
                        Err(operation_error) => {
                            // The mutation may have reached the server. Drop
                            // the connection, and report the result as-is
                            // unless the write is safe to replay.
                            drop(session);
                            if !replay(&operation_error) {
                                return Err(operation_error);
                            }
                            pool_debug(account, "stale-replay");
                        }
                    }
                }
                Err(_) => {
                    // No mutation has run, so replacing this dead pooled
                    // connection and repeating the preflight is safe.
                    drop(session);
                    pool_debug(account, "stale-retry");
                }
            }
        }

        pool_debug(account, "fresh-connect");
        let connect_lock = self.connect_lock(account);
        let mut connect_guard =
            tokio::time::timeout(connect_coordination_timeout(), connect_lock.lock())
                .await
                .ok();
        if connect_guard.is_none() {
            crate::mlog!(
                crate::log::Level::Warn,
                "net",
                "fresh-connect coordination timed out for {account}; connecting independently"
            );
        }
        let pooled_after_wait = self.take_pooled(account);
        if let Some(mut session) = pooled_after_wait {
            drop(connect_guard.take());
            let ready =
                match tokio::time::timeout(POOLED_READ_TIMEOUT, preflight(&mut session)).await {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "pooled session preflight timed out after {}s",
                        POOLED_READ_TIMEOUT.as_secs()
                    )),
                };
            match ready {
                Ok(()) => {
                    let result = f(&mut session).await;
                    match result {
                        Ok(value) => {
                            self.return_pooled(account, session);
                            return Ok(value);
                        }
                        Err(error) => {
                            drop(session);
                            if !replay(&error) {
                                return Err(error);
                            }
                            pool_debug(account, "stale-replay");
                            connect_guard = tokio::time::timeout(
                                connect_coordination_timeout(),
                                connect_lock.lock(),
                            )
                            .await
                            .ok();
                        }
                    }
                }
                Err(_) => {
                    drop(session);
                    pool_debug(account, "stale-retry");
                    connect_guard =
                        tokio::time::timeout(connect_coordination_timeout(), connect_lock.lock())
                            .await
                            .ok();
                }
            }
        }
        let creds = self.ensure_valid_creds(account).await?;
        let connect_started = std::time::Instant::now();
        let mut session = imap::connect(&creds).await?;
        let connect_ms = connect_started.elapsed().as_millis();
        if connect_ms > 2_000 {
            crate::mlog!(
                crate::log::Level::Warn,
                "net",
                "slow IMAP connect for {account}: {connect_ms}ms"
            );
        }
        drop(connect_guard);
        preflight(&mut session).await?;
        match f(&mut session).await {
            Ok(val) => {
                self.return_pooled(account, session);
                Ok(val)
            }
            Err(operation_error) => Err(operation_error),
        }
    }
}
