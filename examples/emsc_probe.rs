//! Protocol probe: connect to the EMSC-CSEM realtime socket with the crate's
//! own WS client and print what comes back. usage: cargo run --example emsc_probe

use serde_json::Value;

#[tokio::main]
async fn main() {
    let mut ws = if let Ok(addr) = std::env::var("TREMOR_WS_PLAIN") {
        let (h, port) = addr.rsplit_once(':').unwrap();
        tremor::ws::WsClient::connect_plain(h, port.parse().unwrap(), "/seismologist", "https://www.emsc-csem.org")
            .await
            .expect("plain connect")
    } else {
        tremor::ws::WsClient::connect(
            tremor::net::EMSC_WS_HOST,
            tremor::net::EMSC_WS_PATH,
            tremor::net::EMSC_WS_ORIGIN,
        )
        .await
        .expect("connect")
    };
    println!("connected + handshake ok");
    ws.send_str(tremor::net::MSG_COMPRESS).await.expect("send compress");
    ws.send_str(tremor::net::MSG_ORIPATH).await.expect("send oripath");
    ws.send_str(&tremor::net::page_req(1)).await.expect("send page1");
    for i in 0..4 {
        let m = match tokio::time::timeout(std::time::Duration::from_secs(25), ws.recv_message()).await {
            Ok(Ok(m)) => m,
            Ok(Err(e)) => { println!("msg {i}: io {e}"); continue; }
            Err(_) => { println!("msg {i}: TIMEOUT"); continue; }
        };
        let b = match m {
            tremor::ws::WsMessage::Closed => { println!("msg {i}: CLOSED"); break; }
            tremor::ws::WsMessage::Text(t) => t.into_bytes(),
            tremor::ws::WsMessage::Binary(b) => {
                if b.starts_with(&[0x1f, 0x8b]) {
                    use std::io::Read;
                    let mut p = Vec::new();
                    if flate2::read::GzDecoder::new(&b[..]).read_to_end(&mut p).is_err() {
                        println!("msg {i}: gzip error"); continue;
                    }
                    p
                } else {
                    b
                }
            }
        };
        let v: Value = serde_json::from_slice(&b).unwrap_or_default();
        let lab = v.get("label").and_then(Value::as_str).unwrap_or("?");
        let n = match lab {
            "full_json" => tremor::net::parse_emsc_page(&v).0.len(),
            _ => 0,
        };
        println!("msg {i}: label={lab} events={n}");
        if lab == "full_json" { break; }
    }
}
