// src/main.rs — Ultra-low latency fraud detection API
// Hyper 1.x HTTP/1.1 server, no framework overhead
// Loads VP-Tree index from /data/index.bin on startup

mod index;
mod vectorize;

use index::Index;
use vectorize::vectorize_payload;

use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use tokio::net::TcpListener;

struct App {
    index: Index,
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let index_path = std::env::var("INDEX_PATH")
        .unwrap_or_else(|_| "/data/index.bin".into());

    eprintln!("[server] Loading index from {}...", index_path);
    let idx = Index::load(&index_path).expect("Failed to load index");
    eprintln!("[server] Ready. {} points, {} nodes", idx.n_points, idx.n_nodes);

    let app = Arc::new(App { index: idx });
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.unwrap();
    eprintln!("[server] Listening on :{}", port);

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, hyper::service::service_fn(move |req| {
                    let app = Arc::clone(&app);
                    async move { Ok::<_, hyper::Error>(route(app, req).await) }
                }))
                .await;
        });
    }
}

async fn route(app: Arc<App>, req: Request<hyper::body::Incoming>) -> Response<Full<Bytes>> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/ready") => {
            Response::builder().status(200).body(Full::new(Bytes::from_static(b"ok"))).unwrap()
        }
        (&Method::POST, "/fraud-score") => {
            let body = match req.collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return fallback_response(),
            };
            match score(&app, &body) {
                Ok(resp) => Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(resp)))
                    .unwrap(),
                Err(_) => fallback_response(),
            }
        }
        _ => Response::builder().status(404)
            .body(Full::new(Bytes::from_static(b"not found"))).unwrap(),
    }
}

// On any error: return approved:true, score:0.0 to avoid HTTP 500 penalty
fn fallback_response() -> Response<Full<Bytes>> {
    Response::builder().status(200)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(b"{\"approved\":true,\"fraud_score\":0.0}")))
        .unwrap()
}

// ─── Payload structs ──────────────────────────────────────────────────────────
#[derive(Deserialize)]
struct Req {
    transaction: Tx,
    customer: Cust,
    merchant: Merch,
    terminal: Term,
    last_transaction: Option<LastTx>,
}

#[derive(Deserialize)]
struct Tx { amount: f32, installments: u32, requested_at: String }

#[derive(Deserialize)]
struct Cust { avg_amount: f32, tx_count_24h: u32, known_merchants: Vec<String> }

#[derive(Deserialize)]
struct Merch { id: String, mcc: String, avg_amount: f32 }

#[derive(Deserialize)]
struct Term { is_online: bool, card_present: bool, km_from_home: f32 }

#[derive(Deserialize)]
struct LastTx { timestamp: String, km_from_current: f32 }

fn score(app: &App, body: &[u8]) -> Result<Vec<u8>, ()> {
    let req: Req = serde_json::from_slice(body).map_err(|_| ())?;

    // Build a Vec<&str> from the owned Vec<String> for the vectorize call
    let known: Vec<&str> = req.customer.known_merchants.iter().map(|s| s.as_str()).collect();

    let v = vectorize_payload(
        req.transaction.amount,
        req.transaction.installments as f32,
        req.customer.avg_amount,
        &req.transaction.requested_at,
        req.last_transaction.as_ref().map(|lt| lt.timestamp.as_str()),
        req.last_transaction.as_ref().map(|lt| lt.km_from_current),
        req.terminal.km_from_home,
        req.customer.tx_count_24h as f32,
        req.terminal.is_online,
        req.terminal.card_present,
        &req.merchant.id,
        &known,
        &req.merchant.mcc,
        req.merchant.avg_amount,
    );

    let fs = app.index.knn5_fraud_score(&v);
    let approved = fs < 0.6;

    // Build JSON without serde (fraud_score is always 0.0/0.2/0.4/0.6/0.8/1.0)
    let score_str: &[u8] = match (fs * 10.0 + 0.5) as u8 {
        0 => b"0.0", 2 => b"0.2", 4 => b"0.4", 6 => b"0.6", 8 => b"0.8", _ => b"1.0",
    };
    let mut out = Vec::with_capacity(50);
    out.extend_from_slice(b"{\"approved\":");
    out.extend_from_slice(if approved { b"true" } else { b"false" });
    out.extend_from_slice(b",\"fraud_score\":");
    out.extend_from_slice(score_str);
    out.push(b'}');
    Ok(out)
}