//! Gate-test mock: speaks just enough Anthropic SSE to exercise the full
//! streaming path through the real app. Serves every connection the same
//! canned response. Never shipped to users — a test bin.
use std::io::{Read, Write};
use std::net::TcpListener;

const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"from the mock\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n";

fn main() {
    let listener = TcpListener::bind("127.0.0.1:47700").expect("bind mock port");
    eprintln!("mock_llm on 127.0.0.1:47700");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            let mut request = Vec::new();
            loop {
                let Ok(n) = stream.read(&mut buf) else { return };
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length = text
                        .lines()
                        .find(|l| l.to_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                SSE.len(),
                SSE
            );
            let _ = stream.write_all(response.as_bytes());
        });
    }
}
