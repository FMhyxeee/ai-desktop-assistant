use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const DEFAULT_MAX_RUNNING_PROCESSES: usize = 16;
const DEFAULT_MAX_LOG_LINES: usize = 2000;
const DEFAULT_MAX_LOG_LINE_CHARS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessStatus {
    Running,
    Exited,
    Stopped,
    Failed,
}

impl ProcessStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
struct ProcessState {
    status: ProcessStatus,
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSnapshot {
    pub id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub status: String,
    pub started_at_unix_ms: u64,
    pub exit_code: Option<i32>,
}

struct ProcessRecord {
    id: String,
    command: String,
    started_at_unix_ms: u64,
    pid: Option<u32>,
    child: Arc<Mutex<Child>>,
    logs: Arc<Mutex<VecDeque<String>>>,
    state: Arc<Mutex<ProcessState>>,
}

#[derive(Clone)]
pub struct ProcessManager {
    records: Arc<Mutex<HashMap<String, Arc<ProcessRecord>>>>,
    max_running_processes: usize,
    max_log_lines: usize,
    max_log_line_chars: usize,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            max_running_processes: DEFAULT_MAX_RUNNING_PROCESSES,
            max_log_lines: DEFAULT_MAX_LOG_LINES,
            max_log_line_chars: DEFAULT_MAX_LOG_LINE_CHARS,
        }
    }

    pub async fn start_process(&self, command: String) -> Result<ProcessSnapshot, String> {
        let command_text = command.trim();
        if command_text.is_empty() {
            return Err("/proc start requires a non-empty command".to_string());
        }

        let running_count = self.running_process_count().await?;
        if running_count >= self.max_running_processes {
            return Err(format!(
                "process limit reached: {}",
                self.max_running_processes
            ));
        }

        let mut command_builder = if cfg!(windows) {
            let mut builder = Command::new("cmd");
            builder.args(["/C", command_text]);
            builder
        } else {
            let mut builder = Command::new("sh");
            builder.args(["-c", command_text]);
            builder
        };

        command_builder
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command_builder
            .spawn()
            .map_err(|err| format!("failed to start process: {err}"))?;

        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let record = Arc::new(ProcessRecord {
            id: uuid::Uuid::new_v4().to_string(),
            command: command_text.to_string(),
            started_at_unix_ms: current_time_millis(),
            pid,
            child: Arc::new(Mutex::new(child)),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(self.max_log_lines))),
            state: Arc::new(Mutex::new(ProcessState {
                status: ProcessStatus::Running,
                exit_code: None,
            })),
        });

        if let Some(stdout) = stdout {
            spawn_log_reader(
                stdout,
                Arc::clone(&record.logs),
                self.max_log_lines,
                self.max_log_line_chars,
                "",
            );
        }

        if let Some(stderr) = stderr {
            spawn_log_reader(
                stderr,
                Arc::clone(&record.logs),
                self.max_log_lines,
                self.max_log_line_chars,
                "[stderr] ",
            );
        }

        self.records
            .lock()
            .await
            .insert(record.id.clone(), Arc::clone(&record));

        self.refresh_record_state(&record).await?;
        self.snapshot_from_record(&record).await
    }

    pub async fn list_processes(&self) -> Result<Vec<ProcessSnapshot>, String> {
        let records = {
            let guard = self.records.lock().await;
            guard.values().cloned().collect::<Vec<_>>()
        };

        let mut snapshots = Vec::with_capacity(records.len());
        for record in records {
            self.refresh_record_state(&record).await?;
            snapshots.push(self.snapshot_from_record(&record).await?);
        }

        snapshots.sort_by_key(|snapshot| snapshot.started_at_unix_ms);
        Ok(snapshots)
    }

    pub async fn process_logs(&self, id: &str, lines: usize) -> Result<Vec<String>, String> {
        let record = self.find_record(id).await?;
        self.refresh_record_state(&record).await?;

        let logs = record.logs.lock().await;
        let requested = lines.max(1).min(self.max_log_lines);
        let len = logs.len();
        let start = len.saturating_sub(requested);
        Ok(logs.iter().skip(start).cloned().collect())
    }

    pub async fn stop_process(&self, id: &str) -> Result<ProcessSnapshot, String> {
        let record = self.find_record(id).await?;
        self.refresh_record_state(&record).await?;

        let should_kill = {
            let state = record.state.lock().await;
            matches!(state.status, ProcessStatus::Running)
        };

        if should_kill {
            let mut child = record.child.lock().await;
            child
                .start_kill()
                .map_err(|err| format!("failed to stop process '{id}': {err}"))?;
            let status = child
                .wait()
                .await
                .map_err(|err| format!("failed waiting process '{id}' to stop: {err}"))?;
            drop(child);

            let mut state = record.state.lock().await;
            state.status = ProcessStatus::Stopped;
            state.exit_code = status.code();
        }

        self.snapshot_from_record(&record).await
    }

    async fn running_process_count(&self) -> Result<usize, String> {
        let records = {
            let guard = self.records.lock().await;
            guard.values().cloned().collect::<Vec<_>>()
        };

        let mut running = 0usize;
        for record in records {
            self.refresh_record_state(&record).await?;
            let state = record.state.lock().await;
            if matches!(state.status, ProcessStatus::Running) {
                running += 1;
            }
        }
        Ok(running)
    }

    async fn find_record(&self, id: &str) -> Result<Arc<ProcessRecord>, String> {
        self.records
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| format!("process not found: {id}"))
    }

    async fn refresh_record_state(&self, record: &Arc<ProcessRecord>) -> Result<(), String> {
        let mut child = record.child.lock().await;
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut state = record.state.lock().await;
                if matches!(state.status, ProcessStatus::Running) {
                    state.status = if status.success() {
                        ProcessStatus::Exited
                    } else {
                        ProcessStatus::Failed
                    };
                    state.exit_code = status.code();
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(err) => Err(format!("failed to query process status: {err}")),
        }
    }

    async fn snapshot_from_record(
        &self,
        record: &Arc<ProcessRecord>,
    ) -> Result<ProcessSnapshot, String> {
        let state = record.state.lock().await;
        Ok(ProcessSnapshot {
            id: record.id.clone(),
            command: record.command.clone(),
            pid: record.pid,
            status: state.status.as_str().to_string(),
            started_at_unix_ms: record.started_at_unix_ms,
            exit_code: state.exit_code,
        })
    }
}

fn spawn_log_reader<R>(
    reader: R,
    logs: Arc<Mutex<VecDeque<String>>>,
    max_lines: usize,
    max_line_chars: usize,
    prefix: &'static str,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut normalized = if prefix.is_empty() {
                line
            } else {
                format!("{prefix}{line}")
            };

            if normalized.chars().count() > max_line_chars {
                let truncated: String = normalized.chars().take(max_line_chars).collect();
                normalized = format!("{truncated}...[truncated]");
            }

            let mut guard = logs.lock().await;
            guard.push_back(normalized);
            while guard.len() > max_lines {
                guard.pop_front();
            }
        }
    });
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
