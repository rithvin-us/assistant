//! Integration tests: boot the real router on an ephemeral port and talk to it
//! over real HTTP and a real WebSocket. Nothing is mocked except the database,
//! which is simply absent.

use std::net::SocketAddr;

use assistant_core::EventBus;
use assistant_protocol::{ClientFrame, HealthStatus, PROTOCOL_VERSION, ServerFrame};
use assistant_server::{app, config::Config};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

const TEST_TOKEN: &str = "integration-test-token";

/// Starts the server on port 0 and returns the address it actually bound to.
///
/// The config is built directly rather than read from the environment: these
/// tests run concurrently in one process, and mutating process-wide environment
/// variables from several threads is both racy and `unsafe`.
async fn spawn() -> SocketAddr {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().expect("valid address"),
        database_url: None,
        dev_auth_token: TEST_TOKEN.to_string(),
        allowed_origins: vec!["http://localhost:1420".to_string()],
        log_filter: "off".to_string(),
    };

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bound");
    let addr = listener.local_addr().expect("local addr");

    let router = app(&config, None, EventBus::default());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("server runs");
    });

    addr
}

#[tokio::test]
async fn health_is_public_and_reports_degraded_without_a_database() {
    let addr = spawn().await;

    let response = reqwest::get(format!("http://{addr}/v1/health"))
        .await
        .expect("request succeeds");
    assert_eq!(response.status(), 200);

    let body: assistant_protocol::HealthResponse = response.json().await.expect("valid body");
    assert_eq!(body.protocol_version, PROTOCOL_VERSION);
    assert_eq!(body.service, "assistant-server");
    assert!(matches!(body.status, HealthStatus::Degraded));
}

#[tokio::test]
async fn conversation_socket_rejects_a_missing_token() {
    let addr = spawn().await;
    let id = uuid::Uuid::new_v4();

    let result =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/conversation/{id}/stream")).await;

    match result {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
        other => panic!("expected an HTTP 401, got {other:?}"),
    }
}

#[tokio::test]
async fn conversation_socket_streams_a_turn() {
    let addr = spawn().await;
    let id = uuid::Uuid::new_v4();

    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{addr}/v1/conversation/{id}/stream?access_token={TEST_TOKEN}"
    ))
    .await
    .expect("socket connects");

    let ready = next_frame(&mut socket).await;
    match ready {
        ServerFrame::Ready {
            conversation_id,
            protocol_version,
        } => {
            assert_eq!(conversation_id, id);
            assert_eq!(protocol_version, PROTOCOL_VERSION);
        }
        other => panic!("expected Ready, got {other:?}"),
    }

    let ping = serde_json::to_string(&ClientFrame::Ping).unwrap();
    socket
        .send(tungstenite::Message::Text(ping.into()))
        .await
        .expect("ping sent");
    assert!(matches!(next_frame(&mut socket).await, ServerFrame::Pong));

    let turn = serde_json::to_string(&ClientFrame::UserText {
        text: "hello".into(),
    })
    .unwrap();
    socket
        .send(tungstenite::Message::Text(turn.into()))
        .await
        .expect("turn sent");

    // The turn must arrive as more than one delta and terminate with TurnEnd,
    // which is what proves the transport is incremental rather than buffered.
    let mut deltas = 0;
    loop {
        match next_frame(&mut socket).await {
            ServerFrame::AssistantDelta { .. } => deltas += 1,
            ServerFrame::TurnEnd { .. } => break,
            other => panic!("unexpected frame {other:?}"),
        }
    }
    assert!(
        deltas >= 2,
        "expected an incremental turn, got {deltas} delta(s)"
    );
}

async fn next_frame<S>(socket: &mut S) -> ServerFrame
where
    S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    loop {
        let message = socket.next().await.expect("stream open").expect("no error");
        if let tungstenite::Message::Text(text) = message {
            return serde_json::from_str(&text).expect("server frame parses");
        }
    }
}
