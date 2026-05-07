use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub const CALLBACK_PORT: u16 = 39014;

/// 啟動 local HTTP server 監聽一次 /callback?token=xxx，
/// 回傳取得的 token。server 收到 token 後自動關閉。
pub async fn start_auth_server() -> tokio::task::JoinHandle<Option<String>> {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT)).await {
            Ok(l) => l,
            Err(_) => return None,
        };

        // 只接受第一個有效請求
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                continue;
            };

            let mut reader = BufReader::new(&mut socket);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).await.is_err() {
                continue;
            }

            // GET /callback?token=xxx HTTP/1.1
            if let Some(token) = extract_token(&request_line) {
                let html = success_html();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = socket.write_all(response.as_bytes()).await;
                return Some(token);
            }

            // 404 for other paths
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        }
    })
}

fn extract_token(request_line: &str) -> Option<String> {
    // "GET /callback?token=xxx HTTP/1.1"
    let path = request_line.split_whitespace().nth(1)?;
    if !path.starts_with("/callback") {
        return None;
    }
    let query = path.split('?').nth(1)?;
    for param in query.split('&') {
        if let Some(val) = param.strip_prefix("token=") {
            let decoded = urlencoding::decode(val).ok()?.into_owned();
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

fn success_html() -> String {
    r#"<!DOCTYPE html>
<html lang="zh-Hant">
<head>
  <meta charset="utf-8">
  <title>登入成功</title>
  <style>
    body { font-family: sans-serif; text-align: center; padding: 60px; background: #f5f5f5; }
    h1 { color: #2e7d32; font-size: 2rem; }
    p  { color: #555; }
  </style>
</head>
<body>
  <h1>登入成功！</h1>
  <p>您可以關閉此視窗，回到上傳工具繼續操作。</p>
  <script>setTimeout(() => window.close(), 2000);</script>
</body>
</html>"#.to_string()
}
