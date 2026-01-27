use embassy_time::Duration;
use picoserve::{
    response::ws,
    routing::{get, get_service},
    AppBuilder, AppRouter,
};

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
                "/ws",
                get(async |upgrade: picoserve::response::WebSocketUpgrade| {
                    upgrade.on_upgrade(WebsocketEcho).with_protocol("echo")
                }),
            )
    }
}

#[embassy_executor::task]
pub async fn web_task(app: &'static AppRouter<WebAppProps>) -> ! {
    log::info!("Web server: Waiting for network...");
    let stack = crate::network::get_network_stack().await;

    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    picoserve::Server::new(app, &config, &mut http_buffer)
        .listen_and_serve(0, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

pub struct WebsocketEcho;

impl ws::WebSocketCallback for WebsocketEcho {
    async fn run<R: picoserve::io::Read, W: picoserve::io::Write<Error = R::Error>>(
        self,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        let mut buffer = [0; 1024];

        let close_reason = loop {
            match rx
                .next_message(&mut buffer, core::future::pending())
                .await?
                .ignore_never_b()
            {
                Ok(ws::Message::Text(data)) => tx.send_text(data).await,
                Ok(ws::Message::Binary(data)) => tx.send_binary(data).await,
                Ok(ws::Message::Close(reason)) => {
                    log::info!("Websocket close reason: {reason:?}");
                    break None;
                }
                Ok(ws::Message::Ping(data)) => tx.send_pong(data).await,
                Ok(ws::Message::Pong(_)) => continue,
                Err(error) => {
                    log::error!("Websocket Error: {error:?}");

                    break Some((error.code(), "Websocket Error"));
                }
            }?;
        };

        tx.close(close_reason).await
    }
}
