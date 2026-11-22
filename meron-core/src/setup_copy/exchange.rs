//! Exchanges setup data after two devices have connected.
//! The session processes peer messages while tracking progress and cancellation.
//! The selected device sends the settings to keep.

use super::*;

pub(super) async fn run(session: Arc<Session>, channel: Channel) {
    let Channel { stream, send, recv } = channel;
    let (mut reader, mut writer) = stream.into_split();
    let (out, mut queue) = mpsc::unbounded_channel::<Value>();
    *session.out.lock().unwrap() = Some(out);
    let mut sealer = Sealer::new(&send);
    let mut opener = Sealer::new(&recv);
    let writing = tokio::spawn(async move {
        while let Some(message) = queue.recv().await {
            let sealed = sealer.seal(message.to_string().as_bytes());
            if write_frame(&mut writer, &sealed).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    session.update(|status| {
        status.stage = "connected";
        status.error.clear();
    });
    let hello = session.snapshot();
    session.send(json!({ "t": "hello", "device": hello.me, "counts": hello.mine }));
    let mut stop = session.stop.subscribe();
    loop {
        let frame = tokio::select! {
            _ = stopped(&mut stop) => break,
            frame = read_frame(&mut reader, MAX_FRAME) => frame,
        };
        let message = frame
            .and_then(|frame| opener.open(frame))
            .and_then(|plain| serde_json::from_slice::<Value>(&plain).map_err(|_| fail("broken")));
        match message {
            Ok(message) => handle(&session, message),
            Err(_) => {
                let stage = session.snapshot().stage;
                // After its answer the other side hangs up: not a failure.
                if !matches!(stage, "done" | "applying" | "failed") {
                    session.fail("peer_left");
                }
                break;
            }
        }
    }
    // Let the writer send what is queued (a goodbye, an answer), then close.
    *session.out.lock().unwrap() = None;
    let _ = tokio::time::timeout(Duration::from_secs(5), writing).await;
}

fn handle(session: &Arc<Session>, message: Value) {
    let kind = message.get("t").and_then(Value::as_str).unwrap_or_default();
    let text = |key: &str| {
        message
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match kind {
        "hello" => {
            let device = text("device");
            let counts: Counts = message
                .get("counts")
                .and_then(|counts| serde_json::from_value(counts.clone()).ok())
                .unwrap_or_default();
            session.update(|status| {
                status.peer_device = device.clone();
                status.peer = Some(counts);
            });
            if session.host {
                // Keep the side with more accounts; the computer on a tie.
                let status = session.snapshot();
                let keep = if counts.accounts > status.mine.accounts {
                    device
                } else if counts.accounts < status.mine.accounts {
                    status.me.clone()
                } else {
                    "computer".to_string()
                };
                session.update(|status| status.keep = keep.clone());
                session.send(json!({ "t": "keep", "keep": keep }));
            }
        }
        "keep" if !session.host => {
            let keep = text("keep");
            session.update(|status| status.keep = keep);
        }
        "want_keep" if session.host => set_keep(&text("keep")),
        "go" if session.host => {
            let status = session.snapshot();
            if status.stage == "connected" && text("keep") == status.keep {
                start_copy(session);
            } else {
                // The direction changed meanwhile: show it, ask again.
                session.send(json!({ "t": "keep", "keep": status.keep }));
                session.send(json!({ "t": "not_started" }));
            }
        }
        "not_started" => session.update(|status| status.starting = false),
        "start" if !session.host => {
            let keep = text("keep");
            session.update(|status| {
                status.keep = keep.clone();
                status.stage = "copying";
                status.starting = false;
            });
            if keep == session.snapshot().me {
                spawn_send_payload(session.clone());
            }
        }
        "data" => {
            let status = session.snapshot();
            if status.stage != "copying" || status.keep == status.me {
                return;
            }
            session.update(|status| status.stage = "applying");
            let payload = message.get("payload").cloned().unwrap_or(Value::Null);
            let session = session.clone();
            tokio::spawn(async move {
                let me = session.snapshot().me;
                match receive(&session.engine, payload, &me).await {
                    Ok(result) => {
                        session.update(|status| {
                            status.stage = "done";
                            status.replaced = true;
                            status.result = Some(result.clone());
                        });
                        session.send(json!({ "t": "applied", "result": result }));
                    }
                    Err(err) => {
                        let code = failure_code(&err);
                        crate::mlog!(
                            crate::log::Level::Warn,
                            "setupCopy",
                            "the copy was not applied: {err:#}"
                        );
                        session.send(json!({ "t": "failed", "error": code }));
                        session.fail(code);
                    }
                }
                session.stop();
            });
        }
        "applied" => {
            let result = message.get("result").cloned();
            session.update(|status| {
                status.stage = "done";
                status.result = result;
            });
            session.stop();
        }
        "failed" => {
            let code = match message.get("error").and_then(Value::as_str) {
                Some("bad_copy") => "peer_bad_copy",
                _ => "peer_failed",
            };
            session.fail(code);
        }
        "bye" => {
            if !matches!(session.snapshot().stage, "done" | "applying") {
                session.fail("peer_left");
            }
        }
        _ => {}
    }
}

pub(super) fn spawn_send_payload(session: Arc<Session>) {
    tokio::spawn(async move {
        let me = session.snapshot().me;
        match collect_payload(&session.engine, &me).await {
            Ok(payload) => match serde_json::to_value(&payload) {
                Ok(value) => session.send(json!({ "t": "data", "payload": value })),
                Err(_) => {
                    session.send(json!({ "t": "failed", "error": "broken" }));
                    session.fail("collect_failed");
                }
            },
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "setupCopy",
                    "could not read this setup: {err:#}"
                );
                session.send(json!({ "t": "failed", "error": "broken" }));
                session.fail("collect_failed");
            }
        }
    });
}
