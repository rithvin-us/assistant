//! Integration tests: boot the real router on an ephemeral port and talk to it
//! over real HTTP and a real WebSocket.
//!
//! Nothing inside the server is stubbed. The socket drives the real
//! orchestrator, which drives the real tool loop and the real permission
//! engine; only the model provider and the tools are deterministic doubles, and
//! the database is simply absent.

use std::{net::SocketAddr, sync::Arc};

use assistant_core::{EventBus, ToolRegistry, testing::EchoTool};
use assistant_models::mock::{MockModelProvider, MockResponse};
use assistant_protocol::{ClientFrame, HealthStatus, PROTOCOL_VERSION, RiskLevel, ServerFrame};
use assistant_server::{app, config::Config, orchestration::Dependencies};
use assistant_tools::{Tool, ToolCall};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

const TEST_TOKEN: &str = "integration-test-token";

fn config() -> Config {
    // Built directly rather than read from the environment: these tests run
    // concurrently in one process, and mutating process-wide environment
    // variables from several threads is both racy and `unsafe`.
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("valid address"),
        database_url: None,
        dev_auth_token: TEST_TOKEN.to_string(),
        allowed_origins: vec!["http://localhost:1420".to_string()],
        log_filter: "off".to_string(),
        max_tool_rounds: 4,
    }
}

/// Starts the server on port 0 and returns the address it actually bound to.
async fn spawn_with(deps: Dependencies) -> SocketAddr {
    spawn_configured(config(), deps).await
}

async fn spawn_configured(config: Config, deps: Dependencies) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bound");
    let addr = listener.local_addr().expect("local addr");

    let router = app(&config, None, EventBus::default(), deps);
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("server runs");
    });

    addr
}

fn registry_of(tools: Vec<Arc<dyn Tool>>) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    Arc::new(registry)
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens an authenticated conversation socket and consumes the `Ready` frame.
async fn connect(addr: SocketAddr) -> (Socket, uuid::Uuid) {
    let id = uuid::Uuid::new_v4();
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{addr}/v1/conversation/{id}/stream?access_token={TEST_TOKEN}"
    ))
    .await
    .expect("socket connects");

    match next_frame(&mut socket).await {
        ServerFrame::Ready {
            conversation_id,
            protocol_version,
        } => {
            assert_eq!(conversation_id, id);
            assert_eq!(protocol_version, PROTOCOL_VERSION);
        }
        other => panic!("expected Ready, got {other:?}"),
    }

    (socket, id)
}

async fn say(socket: &mut Socket, text: &str) {
    let frame = serde_json::to_string(&ClientFrame::UserText { text: text.into() }).unwrap();
    socket
        .send(tungstenite::Message::Text(frame.into()))
        .await
        .expect("turn sent");
}

/// Reads frames until the turn terminates, returning everything received.
///
/// A turn always ends: either `TurnEnd`, or an `Error` frame for a turn that
/// stopped. Anything else means the server dropped the stream, which is a bug.
async fn drain_turn(socket: &mut Socket) -> Vec<ServerFrame> {
    let mut frames = Vec::new();
    loop {
        let frame = next_frame(socket).await;
        let terminal = matches!(frame, ServerFrame::TurnEnd { .. } | ServerFrame::Error(_));
        frames.push(frame);
        if terminal {
            return frames;
        }
    }
}

async fn next_frame(socket: &mut Socket) -> ServerFrame {
    loop {
        let message = socket.next().await.expect("stream open").expect("no error");
        if let tungstenite::Message::Text(text) = message {
            // Parsing as `ServerFrame` is itself the assertion that the server
            // never emits something outside the published protocol.
            return serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("server sent an invalid frame: {e}\n{text}"));
        }
    }
}

fn assistant_text(frames: &[ServerFrame]) -> String {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::AssistantDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ HTTP

#[tokio::test]
async fn health_is_public_and_reports_degraded_without_a_database() {
    let addr = spawn_with(Dependencies::default()).await;

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
    let addr = spawn_with(Dependencies::default()).await;
    let id = uuid::Uuid::new_v4();

    let result =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/conversation/{id}/stream")).await;

    match result {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
        other => panic!("expected an HTTP 401, got {other:?}"),
    }
}

// ------------------------------------------------------------------ transport

#[tokio::test]
async fn ping_is_answered_without_starting_a_turn() {
    let addr = spawn_with(Dependencies::default()).await;
    let (mut socket, _) = connect(addr).await;

    let ping = serde_json::to_string(&ClientFrame::Ping).unwrap();
    socket
        .send(tungstenite::Message::Text(ping.into()))
        .await
        .expect("ping sent");

    assert!(matches!(next_frame(&mut socket).await, ServerFrame::Pong));
}

// ------------------------------------------------------ server -> orchestrator

#[tokio::test]
async fn an_authenticated_socket_drives_the_real_orchestrator() {
    let model = Arc::new(MockModelProvider::always(
        "here is the answer you asked for",
    ));
    let addr = spawn_with(Dependencies {
        model: Some(model.clone()),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "tell me something useful").await;

    let frames = drain_turn(&mut socket).await;

    assert!(
        matches!(frames.last(), Some(ServerFrame::TurnEnd { .. })),
        "the turn did not end cleanly: {frames:?}"
    );
    assert_eq!(assistant_text(&frames), "here is the answer you asked for");
    assert!(
        frames
            .iter()
            .filter(|f| matches!(f, ServerFrame::AssistantDelta { .. }))
            .count()
            > 1,
        "output reached the client all at once instead of streaming"
    );
    assert_eq!(model.calls(), 1);
}

#[tokio::test]
async fn a_tool_turn_streams_proposal_completion_and_end() {
    let model = Arc::new(MockModelProvider::new(vec![
        MockResponse::ToolCalls(vec![ToolCall {
            id: "c1".into(),
            name: "notes.read".into(),
            arguments: serde_json::json!({}),
        }]),
        MockResponse::text("your note says hello"),
    ]));
    let tool = Arc::new(EchoTool::green("notes.read"));

    let addr = spawn_with(Dependencies {
        model: Some(model),
        tools: registry_of(vec![tool.clone()]),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "read my notes").await;

    let frames = drain_turn(&mut socket).await;

    assert!(
        frames.iter().any(|f| matches!(
            f,
            ServerFrame::ToolProposed { name, risk, .. }
                if name == "notes.read" && *risk == RiskLevel::Green
        )),
        "no ToolProposed frame: {frames:?}"
    );
    assert!(
        frames.iter().any(|f| matches!(
            f,
            ServerFrame::ToolCompleted { name, ok, .. } if name == "notes.read" && *ok
        )),
        "no ToolCompleted frame: {frames:?}"
    );
    assert!(matches!(frames.last(), Some(ServerFrame::TurnEnd { .. })));
    assert_eq!(assistant_text(&frames), "your note says hello");
    assert_eq!(tool.calls(), 1);
}

/// Approval stops the turn over the wire, and the tool never runs.
#[tokio::test]
async fn an_approval_required_tool_halts_the_turn_over_the_socket() {
    let model = Arc::new(MockModelProvider::new(vec![MockResponse::ToolCalls(vec![
        ToolCall {
            id: "c1".into(),
            name: "gmail.send".into(),
            // The model asserts the call is harmless. The registry says Red.
            arguments: serde_json::json!({"risk": "green", "requires_approval": false}),
        },
    ])]));
    let dangerous = Arc::new(EchoTool::red("gmail.send"));

    let addr = spawn_with(Dependencies {
        model: Some(model),
        tools: registry_of(vec![dangerous.clone()]),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "email my professor that I will be late").await;

    let frames = drain_turn(&mut socket).await;

    let approval = frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::ApprovalRequired { name, risk, .. } => Some((name.clone(), *risk)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no ApprovalRequired frame: {frames:?}"));
    assert_eq!(approval, ("gmail.send".to_string(), RiskLevel::Red));

    match frames.last() {
        Some(ServerFrame::Error(error)) => assert_eq!(error.code, "approval_required"),
        other => panic!("expected a terminal approval error, got {other:?}"),
    }

    assert!(
        !frames
            .iter()
            .any(|f| matches!(f, ServerFrame::ToolCompleted { .. })),
        "a tool awaiting approval reported completion"
    );
    assert_eq!(dangerous.calls(), 0, "a Red tool ran without approval");
}

/// The latency and cost invariant, proven across the whole stack: a
/// deterministic question travels socket -> orchestrator -> answer without the
/// model provider being invoked once.
#[tokio::test]
async fn a_deterministic_request_is_answered_without_invoking_the_model() {
    let model = Arc::new(MockModelProvider::always("THIS MUST NOT APPEAR"));
    let addr = spawn_with(Dependencies {
        model: Some(model.clone()),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "what is your status?").await;

    let frames = drain_turn(&mut socket).await;
    let text = assistant_text(&frames);

    assert_eq!(model.calls(), 0, "the deterministic path invoked the model");
    assert!(matches!(frames.last(), Some(ServerFrame::TurnEnd { .. })));
    assert!(
        text.contains("Assistant core is running"),
        "unexpected: {text}"
    );
    assert!(!text.contains("THIS MUST NOT APPEAR"));
}

#[tokio::test]
async fn a_deployment_without_a_provider_says_so_rather_than_answering() {
    let addr = spawn_with(Dependencies::default()).await;
    let (mut socket, _) = connect(addr).await;

    say(&mut socket, "write me a study plan").await;
    let frames = drain_turn(&mut socket).await;

    match frames.last() {
        Some(ServerFrame::Error(error)) => {
            assert_eq!(error.code, "no_model_provider");
            assert!(error.message.contains("no model provider"), "{error:?}");
        }
        other => panic!("expected a no_model_provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn the_tool_round_limit_is_enforced_across_the_socket() {
    // A model that asks for the same tool forever.
    let model = Arc::new(
        MockModelProvider::new(vec![]).with_fallback(MockResponse::ToolCalls(vec![ToolCall {
            id: "c1".into(),
            name: "notes.read".into(),
            arguments: serde_json::json!({}),
        }])),
    );
    let tool = Arc::new(EchoTool::green("notes.read"));

    let addr = spawn_configured(
        Config {
            max_tool_rounds: 2,
            ..config()
        },
        Dependencies {
            model: Some(model),
            tools: registry_of(vec![tool.clone()]),
            ..Default::default()
        },
    )
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "go around in circles").await;

    let frames = drain_turn(&mut socket).await;

    match frames.last() {
        Some(ServerFrame::Error(error)) => assert_eq!(error.code, "iteration_limit_exceeded"),
        other => panic!("expected the round limit to stop the turn, got {other:?}"),
    }
    assert_eq!(
        tool.calls(),
        2,
        "the tool ran more often than the limit allows"
    );
}

#[tokio::test]
async fn the_socket_survives_a_failed_turn_and_accepts_the_next_one() {
    let model = Arc::new(
        MockModelProvider::new(vec![MockResponse::Error(
            "transient upstream failure".into(),
        )])
        .with_fallback(MockResponse::text("second time lucky")),
    );
    let addr = spawn_with(Dependencies {
        model: Some(model),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;

    say(&mut socket, "first attempt").await;
    let first = drain_turn(&mut socket).await;
    assert!(matches!(first.last(), Some(ServerFrame::Error(_))));

    say(&mut socket, "second attempt").await;
    let second = drain_turn(&mut socket).await;
    assert!(matches!(second.last(), Some(ServerFrame::TurnEnd { .. })));
    assert_eq!(assistant_text(&second), "second time lucky");
}

// ------------------------------------------------------------------ logging

/// The conversation socket accepts its bearer token as `?access_token=`, because
/// a browser cannot set headers on a WebSocket handshake. That makes the request
/// log a place a live credential can leak, so the span is built from
/// `uri.path()` and never the full URI.
///
/// This asserts the property directly: the path a span would record must not
/// contain the token, even though the request URI does.
#[test]
fn the_logged_request_path_never_contains_the_query_string() {
    let uri: axum::http::Uri = format!(
        "/v1/conversation/11111111-1111-4111-8111-111111111111/stream?access_token={TEST_TOKEN}"
    )
    .parse()
    .expect("valid uri");

    assert!(
        uri.to_string().contains(TEST_TOKEN),
        "the test is not exercising a URI that carries a token"
    );
    assert!(
        !uri.path().contains(TEST_TOKEN),
        "the token reached the value the request span records"
    );
    assert!(!uri.path().contains('?'));
}

// ------------------------------------------------- durable approval round trip

/// Opens a store against `DATABASE_URL`, or `None` when none is configured.
async fn action_store() -> Option<Arc<dyn assistant_core::actions::ActionStore>> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.is_empty())?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");

    Some(Arc::new(assistant_server::store::PostgresActionStore::new(
        pool,
    )))
}

/// The whole point of the milestone, over the wire.
///
/// A `Red` tool is proposed, held, persisted, answered by id over the socket,
/// and executed — with the client never naming the tool or its arguments.
#[tokio::test]
async fn a_held_action_is_approved_over_the_socket_and_then_executes() {
    let Some(store) = action_store().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let model = Arc::new(MockModelProvider::new(vec![MockResponse::ToolCalls(vec![
        ToolCall {
            id: "c1".into(),
            name: "gmail.send".into(),
            arguments: serde_json::json!({"to": "prof@example.edu"}),
        },
    ])]));
    let dangerous = Arc::new(EchoTool::red("gmail.send"));

    let addr = spawn_with(Dependencies {
        model: Some(model),
        tools: registry_of(vec![dangerous.clone()]),
        store: Some(store),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "email my professor").await;
    let frames = drain_turn(&mut socket).await;

    // The action was persisted, so the client is given something to answer.
    let (approval_id, summary) = frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::ApprovalRequired {
                approval_id,
                summary,
                ..
            } => Some((*approval_id, summary.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no ApprovalRequired frame: {frames:?}"));

    let approval_id = approval_id.expect("a durable store must yield an approval id");
    assert!(summary.contains("gmail.send"));
    assert!(
        !summary.contains("prof@example.edu"),
        "an argument value reached the client: {summary}"
    );
    assert_eq!(dangerous.calls(), 0, "the tool ran before approval");

    // Listing shows it as pending.
    let list = serde_json::to_string(&ClientFrame::ListPendingApprovals).unwrap();
    socket
        .send(tungstenite::Message::Text(list.into()))
        .await
        .expect("sent");
    match next_frame(&mut socket).await {
        ServerFrame::PendingApprovals { approvals } => {
            assert!(
                approvals.iter().any(|a| a.approval_id == approval_id),
                "the held action was not listed as pending"
            );
        }
        other => panic!("expected PendingApprovals, got {other:?}"),
    }

    // Approve by id and nothing else.
    let approve = serde_json::to_string(&ClientFrame::ApproveAction { approval_id }).unwrap();
    socket
        .send(tungstenite::Message::Text(approve.into()))
        .await
        .expect("sent");

    match next_frame(&mut socket).await {
        ServerFrame::ApprovalResolved { outcome, .. } => match outcome {
            assistant_protocol::ApprovalOutcome::Executed { ok, .. } => assert!(ok),
            other => panic!("expected execution, got {other:?}"),
        },
        other => panic!("expected ApprovalResolved, got {other:?}"),
    }
    assert_eq!(
        dangerous.calls(),
        1,
        "the approved tool did not run exactly once"
    );

    // A second tap must not run it again.
    let again = serde_json::to_string(&ClientFrame::ApproveAction { approval_id }).unwrap();
    socket
        .send(tungstenite::Message::Text(again.into()))
        .await
        .expect("sent");
    match next_frame(&mut socket).await {
        ServerFrame::ApprovalResolved { outcome, .. } => assert!(
            matches!(
                outcome,
                assistant_protocol::ApprovalOutcome::AlreadyResolved { .. }
            ),
            "a second tap was not recognised as a duplicate: {outcome:?}"
        ),
        other => panic!("expected ApprovalResolved, got {other:?}"),
    }
    assert_eq!(dangerous.calls(), 1, "a double tap executed the tool twice");
}

/// Rejecting over the socket must never execute.
#[tokio::test]
async fn a_held_action_rejected_over_the_socket_never_executes() {
    let Some(store) = action_store().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let model = Arc::new(MockModelProvider::new(vec![MockResponse::ToolCalls(vec![
        ToolCall {
            id: "c1".into(),
            name: "gmail.send".into(),
            arguments: serde_json::json!({"to": "prof@example.edu"}),
        },
    ])]));
    let dangerous = Arc::new(EchoTool::red("gmail.send"));

    let addr = spawn_with(Dependencies {
        model: Some(model),
        tools: registry_of(vec![dangerous.clone()]),
        store: Some(store),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    say(&mut socket, "email my professor").await;
    let frames = drain_turn(&mut socket).await;

    let approval_id = frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::ApprovalRequired { approval_id, .. } => *approval_id,
            _ => None,
        })
        .expect("an approval id");

    let reject = serde_json::to_string(&ClientFrame::RejectAction { approval_id }).unwrap();
    socket
        .send(tungstenite::Message::Text(reject.into()))
        .await
        .expect("sent");

    match next_frame(&mut socket).await {
        ServerFrame::ApprovalResolved { outcome, .. } => assert!(
            matches!(
                outcome,
                assistant_protocol::ApprovalOutcome::Rejected { .. }
            ),
            "expected rejection, got {outcome:?}"
        ),
        other => panic!("expected ApprovalResolved, got {other:?}"),
    }
    assert_eq!(dangerous.calls(), 0, "a rejected action executed");
}

/// An approval belonging to somebody else is reported as missing, not forbidden.
#[tokio::test]
async fn answering_an_unknown_approval_reports_not_found() {
    let Some(store) = action_store().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };

    let addr = spawn_with(Dependencies {
        store: Some(store),
        ..Default::default()
    })
    .await;

    let (mut socket, _) = connect(addr).await;
    let approve = serde_json::to_string(&ClientFrame::ApproveAction {
        approval_id: uuid::Uuid::new_v4(),
    })
    .unwrap();
    socket
        .send(tungstenite::Message::Text(approve.into()))
        .await
        .expect("sent");

    match next_frame(&mut socket).await {
        ServerFrame::ApprovalResolved { outcome, .. } => assert!(
            matches!(outcome, assistant_protocol::ApprovalOutcome::NotFound),
            "expected NotFound, got {outcome:?}"
        ),
        other => panic!("expected ApprovalResolved, got {other:?}"),
    }
}
