use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use uuid::Uuid;

use crate::state::StateStore;
use crate::types::api::LogEventResponse;

/// Stream logs for a loop via SSE.
///
/// For active loops: tails from Postgres, sending new events as they appear.
/// Closes when the loop reaches a terminal state.
pub async fn stream_logs(
    store: Arc<dyn StateStore>,
    loop_id: Uuid,
    round: Option<i32>,
    stage: Option<String>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream = async_stream::stream! {
        let mut last_timestamp = chrono::DateTime::<chrono::Utc>::MIN_UTC;
        let poll_interval = Duration::from_millis(500);

        loop {
            // Get new logs since last timestamp
            let logs = match store.get_logs_after(loop_id, last_timestamp).await {
                Ok(logs) => logs,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to get logs for SSE");
                    break;
                }
            };

            for log in &logs {
                // Apply filters
                if round.is_some_and(|r| log.round != r) {
                    continue;
                }
                if stage.as_ref().is_some_and(|s| log.stage != *s) {
                    continue;
                }

                let event = LogEventResponse {
                    timestamp: log.timestamp,
                    stage: log.stage.clone(),
                    round: log.round,
                    line: log.line.clone(),
                };

                if let Ok(json) = serde_json::to_string(&event) {
                    yield Ok(Event::default().data(json));
                }

                if log.timestamp > last_timestamp {
                    last_timestamp = log.timestamp;
                }
            }

            // Check if loop is terminal
            match store.get_loop(loop_id).await {
                Ok(Some(record)) if record.state.is_terminal() => {
                    // Send final event and close
                    yield Ok(Event::default().data(
                        serde_json::json!({
                            "type": "end",
                            "state": record.state,
                        }).to_string()
                    ));
                    break;
                }
                Ok(None) => break,
                Err(_) => break,
                _ => {}
            }

            tokio::time::sleep(poll_interval).await;
        }
    };

    Sse::new(stream)
}
