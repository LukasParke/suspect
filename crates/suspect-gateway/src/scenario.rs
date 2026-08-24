//! Ordered scenario serving.
//!
//! A [`Scenario`] is a scripted sequence of expected requests. Each
//! matching request consumes the next step and receives the step's canned
//! response; a request that does not match the pending step gets `400`
//! problem+json naming expected versus got; requests after the last step
//! get `410 Gone`. This powers deterministic multi-step tests today and
//! CLI-driven scenario runs later.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Request as AxumRequest, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::problem;

/// One scripted step: the request to expect and the response to give.
#[derive(Debug, Clone, PartialEq)]
pub struct StepExpect {
    /// Expected HTTP method (case-insensitive match).
    pub method: String,
    /// Suffix the request path must end with (`/pets/42` matches both
    /// `/pets/42` and `/api/pets/42`).
    pub path_suffix: String,
    /// Status code served on a match.
    pub status: u16,
    /// JSON body served on a match.
    pub body: serde_json::Value,
}

/// An ordered script of steps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scenario {
    /// Steps in consumption order.
    pub steps: Vec<StepExpect>,
}

/// Shared scenario state: remaining steps plus consumed-step cursor.
struct ScenarioCore {
    steps: tokio::sync::Mutex<Vec<StepExpect>>,
    cursor: AtomicUsize,
}

/// Serves `steps` in order on `127.0.0.1:port`, blocking until the server
/// fails.
///
/// # Errors
/// Bind or server failures, rendered as strings.
pub async fn serve_scenario(port: u16, steps: Vec<StepExpect>) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("cannot bind 127.0.0.1:{port}: {e}"))?;
    serve_scenario_on(listener, steps).await
}

/// Serves a scenario on an already-bound listener.
///
/// Binding separately (e.g. port `0`) lets tests discover the actual
/// address before traffic starts.
///
/// # Errors
/// Server failures, rendered as strings.
pub async fn serve_scenario_on(
    listener: tokio::net::TcpListener,
    steps: Vec<StepExpect>,
) -> Result<(), String> {
    axum::serve(listener, scenario_router(steps))
        .await
        .map_err(|e| format!("scenario server error: {e}"))
}

/// Builds the scenario router without binding a socket.
pub fn scenario_router(steps: Vec<StepExpect>) -> Router {
    let core = Arc::new(ScenarioCore {
        steps: tokio::sync::Mutex::new(steps),
        cursor: AtomicUsize::new(0),
    });
    Router::new().fallback(consume_step).with_state(core)
}

/// The single fallback handler: every request goes through step matching.
async fn consume_step(State(core): State<Arc<ScenarioCore>>, request: AxumRequest) -> Response {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();

    let steps = core.steps.lock().await;
    let idx = core.cursor.load(Ordering::SeqCst);
    if idx >= steps.len() {
        return problem(
            StatusCode::GONE,
            "Scenario exhausted",
            Some(format!("all {} steps were consumed", steps.len())),
        );
    }
    let step = &steps[idx];
    if method.eq_ignore_ascii_case(&step.method) && path.ends_with(&step.path_suffix) {
        core.cursor.fetch_add(1, Ordering::SeqCst);
        let status = StatusCode::from_u16(step.status).unwrap_or(StatusCode::OK);
        (
            status,
            [("content-type", "application/json")],
            step.body.to_string(),
        )
            .into_response()
    } else {
        problem(
            StatusCode::BAD_REQUEST,
            "Step mismatch",
            Some(format!(
                "expected {} *{}, got {} {path}",
                step.method, step.path_suffix, method
            )),
        )
    }
}
