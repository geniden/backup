//! Background WebSocket reader with ping/pong handling.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::manager::SharedSplitSink;

pub type WsRead = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
>;

pub async fn graceful_close(read: &mut WsRead, write: &SharedSplitSink) {
    if write.lock().await.close().await.is_err() {
        return;
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    })
    .await;
}

pub async fn close_write_half(write: &SharedSplitSink) {
    let _ = write.lock().await.close().await;
}
pub fn is_app_ping(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|json| json.get("type").and_then(|v| v.as_str()).map(|t| t == "ping"))
        .unwrap_or(false)
}

pub async fn reply_app_pong(write: &SharedSplitSink) -> anyhow::Result<()> {
    write
        .lock()
        .await
        .send(Message::Text(r#"{"type":"pong"}"#.into()))
        .await?;
    Ok(())
}

pub struct InboundPump {
    reader: JoinHandle<anyhow::Result<()>>,
    rx: mpsc::Receiver<String>,
}

impl InboundPump {
    pub fn start(
        mut read: futures::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        write: SharedSplitSink,
        on_disconnect: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let write_ping = write.clone();

        let reader = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if is_app_ping(&text) {
                            reply_app_pong(&write_ping).await?;
                            continue;
                        }
                        if tx.send(text).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Ping(payload)) => {
                        write_ping
                            .lock()
                            .await
                            .send(Message::Pong(payload))
                            .await?;
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!("WebSocket read ended: {e}");
                        break;
                    }
                }
            }
            on_disconnect();
            Ok(())
        });

        Self { reader, rx }
    }

    pub async fn recv(&mut self) -> Option<String> {
        self.rx.recv().await
    }

    pub async fn finish(self) -> anyhow::Result<()> {
        self.reader.await??;
        Ok(())
    }

    pub async fn shutdown(self, write: &SharedSplitSink) {
        close_write_half(write).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), self.reader).await;
    }
}
