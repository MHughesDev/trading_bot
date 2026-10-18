//! A local HTTP stand-in for venue APIs, shared by the adapter tests.
//!
//! Answers canned bodies per `METHOD /path` (responses served in order, the
//! last repeating) and records every request head, so a test can assert on
//! what an adapter *sent* — not only on what it parsed.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Routes = HashMap<String, Vec<(u16, String)>>;

pub struct Mock {
    pub base: String,
    pub heads: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    pub fn hits(&self, route: &str) -> usize {
        self.heads
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.starts_with(&format!("{route} ")) || h.starts_with(&format!("{route}?")))
            .count()
    }

    pub fn head_containing(&self, route: &str) -> String {
        self.heads
            .lock()
            .unwrap()
            .iter()
            .find(|h| h.starts_with(route))
            .cloned()
            .unwrap_or_default()
    }
}

pub async fn serve(routes: Vec<(&str, Vec<(u16, &str)>)>) -> Mock {
    let table: Arc<Mutex<Routes>> = Arc::new(Mutex::new(
        routes
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into_iter().map(|(s, b)| (s, b.to_owned())).collect()))
            .collect(),
    ));
    let heads = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (t2, h2) = (Arc::clone(&table), Arc::clone(&heads));
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { continue };
            let (table, heads) = (Arc::clone(&t2), Arc::clone(&h2));
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the head, then any body the Content-Length promises.
                let (head_end, content_len) = loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (p + 4, cl);
                    }
                };
                while buf.len() < head_end + content_len {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                let path = target.split('?').next().unwrap_or("");
                let key = format!("{method} {path}");
                heads
                    .lock()
                    .unwrap()
                    .push(format!("{key} {}", head.replace("\r\n", " | ")));
                let (status, body) = {
                    let mut t = table.lock().unwrap();
                    match t.get_mut(&key) {
                        Some(q) if q.len() > 1 => q.remove(0),
                        Some(q) if q.len() == 1 => q[0].clone(),
                        _ => (404, "{}".to_owned()),
                    }
                };
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    Mock { base, heads }
}

