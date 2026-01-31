use embassy_time::Duration;
use heapless::Vec;
use picoserve::{
    response::ws,
    routing::{get, get_service},
    AppBuilder, AppRouter,
};

use core::cell::Cell;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;

use crate::draw_app::{WS_CONNECTION_STATE, WS_DATA_CHANNEL};

static WS_ACTIVE: Mutex<CriticalSectionRawMutex, Cell<bool>> = Mutex::new(Cell::new(false));

pub struct WebAppProps;

impl AppBuilder for WebAppProps {
    type PathRouter = impl picoserve::routing::PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        picoserve::Router::new()
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!(
                    "www/index.html"
                ))),
            )
            .route(
                "/index.css",
                get_service(picoserve::response::File::css(include_str!(
                    "www/index.css"
                ))),
            )
            .route(
                "/index.js",
                get_service(picoserve::response::File::javascript(include_str!(
                    "www/index.js"
                ))),
            )
            .route(
                "/draw",
                get(async |upgrade: picoserve::response::WebSocketUpgrade| {
                    log::info!("WebSocket draw: upgrade request received");
                    upgrade.on_upgrade(WebsocketDraw)
                }),
            )
    }
}

#[embassy_executor::task(pool_size = 2)]
pub async fn web_task(id: usize, app: &'static AppRouter<WebAppProps>) -> ! {
    log::info!("Web server {}: Waiting for network...", id);
    let stack = crate::network::get_network_stack().await;
    log::info!("Web server {}: Listening on port 80", id);

    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(2)),
        persistent_start_read_request: Some(Duration::from_millis(500)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    });

    picoserve::Server::new(app, &config, &mut http_buffer)
        .listen_and_serve(id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

pub struct WebsocketDraw;

impl ws::WebSocketCallback for WebsocketDraw {
    async fn run<R: picoserve::io::Read, W: picoserve::io::Write<Error = R::Error>>(
        self,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        // Reject if another WebSocket is already active
        let already_active = WS_ACTIVE.lock(|c| {
            if c.get() {
                true
            } else {
                c.set(true);
                false
            }
        });
        if already_active {
            log::warn!("WebSocket draw: already in use, rejecting");
            return tx.close((1013u16, "Already in use")).await;
        }

        log::info!("WebSocket draw: connection accepted");
        // Set connection state to true - this triggers app switch
        WS_CONNECTION_STATE.sender().send(true);

        crate::mqtt::MqttMessage::enqueue_state(
            crate::mqtt::topics::WEBSOCKET_STATE_TOPIC,
            "connected",
        )
        .await;

        let mut buffer = [0u8; 2048];
        loop {
            match rx
                .next_message(&mut buffer, core::future::pending())
                .await?
                .ignore_never_b()
            {
                Ok(ws::Message::Binary(data)) => {
                    // Forward raw bytes to WebAppRunner
                    let mut vec = Vec::new();
                    if vec.extend_from_slice(data).is_err() {
                        log::warn!("WebSocket draw: message too large, dropping");
                    } else {
                        WS_DATA_CHANNEL.send(vec).await;
                    }
                }
                Ok(ws::Message::Close(reason)) => {
                    log::info!("WebSocket draw close: {reason:?}");
                    break;
                }
                Ok(ws::Message::Ping(data)) => tx.send_pong(data).await?,
                Ok(ws::Message::Text(_)) => {
                    // Ignore text messages - we only handle binary
                    log::warn!("WebSocket draw: received text message, ignoring");
                }
                Ok(ws::Message::Pong(_)) => continue,
                Err(error) => {
                    log::error!("WebSocket draw error: {error:?}");
                    break;
                }
            }
        }

        // Set connection state to false on disconnect
        WS_ACTIVE.lock(|c| c.set(false));
        WS_CONNECTION_STATE.sender().send(false);

        crate::mqtt::MqttMessage::enqueue_state(
            crate::mqtt::topics::WEBSOCKET_STATE_TOPIC,
            "disconnected",
        )
        .await;

        tx.close(None).await
    }
}
