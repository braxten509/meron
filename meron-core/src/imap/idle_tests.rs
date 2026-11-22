//! Notification coalescing on an idle connection.
//!
//! Drives an IDLE session with a local server sending mailbox updates.
//! Checks how bursts of removals and an incoming message affect the wait.

use super::*;
use async_imap::extensions::idle::IdleResponse;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

// A bulk delete reaches the watcher as one removal notice per message.
// The watcher must wait the burst out (one folder re-read, not one per
// notice), yet act at once on a notice of new mail.
#[tokio::test]
async fn a_burst_of_removals_is_waited_out_but_new_mail_is_not() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let removals_sent = Arc::new(AtomicUsize::new(0));
    let sent = removals_sent.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        writer.write_all(b"* OK IMAP4rev1 ready\r\n").await.unwrap();
        let mut lines = tokio::io::BufReader::new(reader).lines();
        let mut idles = 0;
        let mut idle_tag = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line == "DONE" {
                let reply = format!("{idle_tag} OK done\r\n");
                writer.write_all(reply.as_bytes()).await.unwrap();
                continue;
            }
            let (tag, command) = line.split_once(' ').unwrap();
            if command != "IDLE" {
                let reply = format!("{tag} OK done\r\n");
                writer.write_all(reply.as_bytes()).await.unwrap();
                continue;
            }
            idle_tag = tag.to_string();
            idles += 1;
            writer.write_all(b"+ idling\r\n").await.unwrap();
            if idles == 1 {
                // First IDLE: three removals, 60 ms apart.
                for _ in 0..3 {
                    writer.write_all(b"* 5 EXPUNGE\r\n").await.unwrap();
                    sent.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(60)).await;
                }
            } else {
                // Second IDLE: new mail.
                writer.write_all(b"* 6 EXISTS\r\n").await.unwrap();
            }
        }
    });
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut client = async_imap::Client::new(Stream::plain(tcp));
    client.read_response().await.unwrap().unwrap();
    let session = client
        .login("test", "test")
        .await
        .map_err(|(err, _)| err)
        .unwrap();
    let (quiet, cap) = (Duration::from_millis(200), Duration::from_secs(2));

    let mut handle = session.idle();
    handle.init().await.unwrap();
    let first = {
        let (wait, _stop) = handle.wait_with_timeout(Duration::from_secs(2));
        wait.await.unwrap()
    };
    assert!(matches!(first, IdleResponse::NewData(_)));
    settle_idle(&mut handle, &first, quiet, cap).await.unwrap();
    assert_eq!(
        removals_sent.load(Ordering::SeqCst),
        3,
        "the wait must outlast the whole burst"
    );
    let session = handle.done().await.unwrap();

    let mut handle = session.idle();
    handle.init().await.unwrap();
    let first = {
        let (wait, _stop) = handle.wait_with_timeout(Duration::from_secs(2));
        wait.await.unwrap()
    };
    let started = Instant::now();
    settle_idle(&mut handle, &first, quiet, cap).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "new mail must not be held back"
    );
    drop(handle);
    task.abort();
}
