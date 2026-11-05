//! A scripted [`Transport`] for tests: answers in order, records what was asked.

use std::cell::RefCell;

use anyhow::Result;
use serde_json::{Value, json};

use super::{Method, Response, Transport};

/// Answers from a script and records what was asked.
pub(crate) struct Mock {
    script: RefCell<Vec<Result<Response>>>,
    calls: RefCell<Vec<(Method, String, Option<String>)>>,
}

impl Mock {
    pub(crate) fn new(script: Vec<Result<Response>>) -> Self {
        Self {
            script: RefCell::new(script.into_iter().rev().collect()),
            calls: RefCell::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(Method, String, Option<String>)> {
        self.calls.borrow().clone()
    }

    pub(crate) fn methods(&self) -> Vec<Method> {
        self.calls().into_iter().map(|c| c.0).collect()
    }
}

impl Transport for Mock {
    fn send(&self, method: Method, path: &str, body: Option<&str>) -> Result<Response> {
        self.calls
            .borrow_mut()
            .push((method, path.to_string(), body.map(String::from)));
        self.script
            .borrow_mut()
            .pop()
            .expect("an unexpected request")
    }
}

pub(crate) fn ok(status: u16, body: &str) -> Result<Response> {
    Ok(Response {
        status,
        body: body.to_string(),
    })
}

pub(crate) fn page(comments: &[(u64, &str)]) -> Result<Response> {
    let items: Vec<Value> = comments
        .iter()
        .map(|(id, body)| json!({ "id": id, "body": body }))
        .collect();
    ok(200, &Value::Array(items).to_string())
}

thread_local! {
    /// Where this thread's log lines go while it is inside [`logged`].
    static CAPTURE: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes a log line to the buffer of the thread that logged it, if that thread
/// is inside [`logged`], and drops it otherwise.
#[derive(Clone, Copy)]
struct ToCapture;

impl std::io::Write for ToCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        CAPTURE.with(|c| {
            if let Some(buf) = c.borrow().as_ref() {
                buf.lock().expect("lock").extend_from_slice(bytes);
            }
        });
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Run `f`, returning what it logged along with its result.
///
/// One subscriber is installed for the whole test process and sends each
/// thread's lines to that thread's buffer. A subscriber installed per call with
/// `tracing::subscriber::with_default` lost a line in about 3 runs in 100 of the
/// whole suite (a warning the capturing test had caused was missing from its
/// log); tracing decides per log statement whether anyone wants it and caches
/// the answer, and parallel tests changing the subscribers of their threads at
/// the same time are the likely cause. With one subscriber that wants
/// everything the failure did not occur in 600 runs.
pub(crate) fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
    use std::sync::{Arc, Mutex, Once};

    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(|| ToCapture)
            .with_ansi(false)
            .finish();
        // A second global subscriber is not an error worth stopping a test for.
        let _ = tracing::subscriber::set_global_default(subscriber);
    });

    /// Stops capturing when dropped, so a panic in `f` does not leave the
    /// buffer attached to the thread.
    struct Detach;
    impl Drop for Detach {
        fn drop(&mut self) {
            CAPTURE.with(|c| c.borrow_mut().take());
        }
    }

    let buf = Arc::new(Mutex::new(Vec::new()));
    CAPTURE.with(|c| *c.borrow_mut() = Some(Arc::clone(&buf)));
    let detach = Detach;
    let result = f();
    drop(detach);
    let text = String::from_utf8(buf.lock().expect("lock").clone()).expect("utf8");
    (result, text)
}

/// Serves `answers` in order, one request each, on a local port; returns the
/// base URL and, once joined, the requests it received (head and body).
pub(crate) fn serve(
    answers: Vec<(&'static str, String)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    serve_raw(
        answers
            .into_iter()
            .map(|(status_line, answer)| {
                format!(
                    "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                )
            })
            .collect(),
    )
}

/// Like [`serve`], but each reply is the whole raw HTTP response, so a test can
/// send one that is wrong (a body shorter than its Content-Length).
pub(crate) fn serve_raw(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).expect("read");
                received.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let wanted = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().expect("length"))
                        })
                        .unwrap_or(0);
                    if body.len() >= wanted {
                        break;
                    }
                }
                assert!(n > 0, "the client hung up early");
            }
            stream.write_all(reply.as_bytes()).expect("reply");
            requests.push(String::from_utf8_lossy(&received).to_string());
        }
        requests
    });
    (base, handle)
}
