use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_desktop_assistant_lib::agent_service::types::{AgentEvent, AgentStreamInput};
use ai_desktop_assistant_lib::agent_service::{AgentService, Runner};

struct MockRunner {
    output: String,
    delay_ms: u64,
}

#[async_trait::async_trait]
impl Runner for MockRunner {
    async fn run(&self, _prompt: &str) -> agent_lib::AgentResult<String> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        Ok(self.output.clone())
    }
}

#[tokio::test]
async fn chat_returns_runner_output() {
    let runner = Arc::new(MockRunner {
        output: "ok".to_string(),
        delay_ms: 0,
    });
    let service = AgentService::with_runner(runner);
    let result = service.chat("hello".to_string()).await.unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn chat_stream_emits_lifecycle_events() {
    let runner = Arc::new(MockRunner {
        output: "stream-output".to_string(),
        delay_ms: 0,
    });
    let service = AgentService::with_runner(runner);

    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            "task-1".to_string(),
            AgentStreamInput::text("hi"),
            move |event| {
            events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let events = events.lock().unwrap();
    assert!(matches!(events.first(), Some(AgentEvent::Started { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::Completed { .. })));
}

#[tokio::test]
async fn cancel_existing_task_succeeds() {
    let runner = Arc::new(MockRunner {
        output: "late-output".to_string(),
        delay_ms: 800,
    });
    let service = AgentService::with_runner(runner);

    service
        .chat_stream(
            "task-2".to_string(),
            AgentStreamInput::text("hi"),
            |_event| {},
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    service.cancel("task-2").await.unwrap();
}
