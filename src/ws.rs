use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::AppState;

/// WebSocket message types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    /// Subscribe to plume updates for a specific region
    Subscribe { region: String },
    /// Unsubscribe from updates
    Unsubscribe { region: String },
    /// Plume update notification
    PlumeUpdate {
        id: String,
        emission_rate_kg_hr: f64,
        lat: f64,
        lon: f64,
        timestamp: String,
    },
    /// Weather update notification
    WeatherUpdate {
        region: String,
        wind_speed_ms: f64,
        wind_direction_deg: f64,
        temperature_c: f64,
        humidity_percent: f64,
    },
    /// Alert notification
    Alert {
        zone_name: String,
        region: String,
        emission_rate_kg_hr: f64,
        message: String,
    },
    /// Heartbeat
    Ping,
    /// Heartbeat response
    Pong,
}

/// WebSocket state
pub struct WsState {
    pub tx: broadcast::Sender<WsMessage>,
}

impl WsState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self { tx }
    }
}

/// WebSocket upgrade handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    
    // Subscribe to broadcast channel
    let mut rx = state.ws_state.tx.subscribe();
    
    // Spawn task to forward broadcast messages to WebSocket
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(Message::Text(json)).await;
            }
        }
    });
    
    // Clone state for receive task
    let state_clone = state.clone();
    let tx = state_clone.ws_state.tx.clone();
    
    // Spawn task to handle incoming WebSocket messages
    let mut recv_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(msg) => {
                    match msg {
                        Message::Text(text) => {
                            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                match ws_msg {
                                    WsMessage::Ping => {
                                        let _ = tx.send(WsMessage::Pong);
                                    }
                                    WsMessage::Subscribe { region } => {
                                        tracing::info!("Client subscribed to region: {}", region);
                                    }
                                    WsMessage::Unsubscribe { region } => {
                                        tracing::info!("Client unsubscribed from region: {}", region);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                Err(_) => break,
            }
        }
    });
    
    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
        }
        _ = &mut recv_task => {
            send_task.abort();
        }
    }
}

/// Broadcast a plume update to all connected clients
pub async fn broadcast_plume_update(
    tx: &broadcast::Sender<WsMessage>,
    id: String,
    emission_rate_kg_hr: f64,
    lat: f64,
    lon: f64,
    timestamp: String,
) {
    let msg = WsMessage::PlumeUpdate {
        id,
        emission_rate_kg_hr,
        lat,
        lon,
        timestamp,
    };
    let _ = tx.send(msg);
}

/// Broadcast a weather update to all connected clients
pub async fn broadcast_weather_update(
    tx: &broadcast::Sender<WsMessage>,
    region: String,
    wind_speed_ms: f64,
    wind_direction_deg: f64,
    temperature_c: f64,
    humidity_percent: f64,
) {
    let msg = WsMessage::WeatherUpdate {
        region,
        wind_speed_ms,
        wind_direction_deg,
        temperature_c,
        humidity_percent,
    };
    let _ = tx.send(msg);
}

/// Broadcast an alert to all connected clients
pub async fn broadcast_alert(
    tx: &broadcast::Sender<WsMessage>,
    zone_name: String,
    region: String,
    emission_rate_kg_hr: f64,
    message: String,
) {
    let msg = WsMessage::Alert {
        zone_name,
        region,
        emission_rate_kg_hr,
        message,
    };
    let _ = tx.send(msg);
}
