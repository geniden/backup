//! Registry of active WebSocket write halves (one entry per server URL).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::WebSocketStream;

pub type SharedSplitSink = Arc<
    Mutex<
        futures::stream::SplitSink<
            WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            tokio_tungstenite::tungstenite::Message,
        >,
    >,
>;

pub struct ConnectionManager {
    sinks: Mutex<HashMap<String, SharedSplitSink>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            sinks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn insert(&self, url: &str, sink: SharedSplitSink) {
        self.sinks.lock().await.insert(url.to_string(), sink);
    }

    pub async fn get(&self, url: &str) -> Option<SharedSplitSink> {
        self.sinks.lock().await.get(url).cloned()
    }

    pub async fn remove(&self, url: &str) {
        self.sinks.lock().await.remove(url);
    }
}
