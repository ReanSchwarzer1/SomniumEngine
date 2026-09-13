//! Session-qualified feedback requests and terminal receipts. Completion means
//! the host has published real renderer/import output, never just queued work.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

const CAPACITY: usize = 128;
const MAX_PENDING: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Capture,
    Import,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Queued,
    Running,
    AwaitingPublish,
    Succeeded,
    Failed,
    Cancelled,
}

impl FeedbackStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackRequest {
    pub job_id: String,
    pub request_id: String,
    pub kind: FeedbackKind,
    pub params: Value,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackReceipt {
    pub job_id: String,
    pub request_id: String,
    pub kind: FeedbackKind,
    pub status: FeedbackStatus,
    pub progress: f32,
    pub expected_revision: u64,
    pub published_revision: Option<u64>,
    pub cancellation_requested: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

struct Entry {
    request: FeedbackRequest,
    receipt: FeedbackReceipt,
}

/// Bounded in-memory journal. Adapter reconnection retains it; editor restart
/// creates a different session and requires fresh queries. It is not a durable
/// MCP Tasks implementation and does not advertise that extension.
pub struct FeedbackQueue {
    session: String,
    sequence: u64,
    entries: BTreeMap<String, Entry>,
    order: VecDeque<String>,
    pending: VecDeque<String>,
}

impl FeedbackQueue {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session: session_id.into(),
            sequence: 0,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            pending: VecDeque::new(),
        }
    }

    /// Deduplicate identical request IDs, reject reuse with different inputs.
    pub fn submit(
        &mut self,
        request_id: &str,
        kind: FeedbackKind,
        params: Value,
        revision: u64,
    ) -> Result<FeedbackReceipt, String> {
        if request_id.is_empty() || request_id.len() > 256 || !params.is_object() {
            return Err("request_id must contain 1–256 bytes and params must be an object".into());
        }
        if let Some(entry) = self
            .entries
            .values()
            .find(|entry| entry.request.request_id == request_id)
        {
            if entry.request.kind != kind
                || entry.request.params != params
                || entry.request.expected_revision != revision
            {
                return Err("request_id was already used with different inputs".into());
            }
            return Ok(entry.receipt.clone());
        }
        if self
            .entries
            .values()
            .filter(|entry| !entry.receipt.status.terminal())
            .count()
            >= MAX_PENDING
        {
            return Err("feedback queue is full; wait for or cancel an existing job".into());
        }
        while self.entries.len() >= CAPACITY {
            let Some(index) = self
                .order
                .iter()
                .position(|id| self.entries[id].receipt.status.terminal())
            else {
                return Err("feedback receipt journal is full".into());
            };
            if let Some(id) = self.order.remove(index) {
                self.entries.remove(&id);
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("feedback sequence exhausted")?;
        let job_id = format!("{}:feedback:{}", self.session, self.sequence);
        let request = FeedbackRequest {
            job_id: job_id.clone(),
            request_id: request_id.into(),
            kind,
            params,
            expected_revision: revision,
        };
        let receipt = FeedbackReceipt {
            job_id: job_id.clone(),
            request_id: request_id.into(),
            kind,
            status: FeedbackStatus::Queued,
            progress: 0.0,
            expected_revision: revision,
            published_revision: None,
            cancellation_requested: false,
            result: None,
            error: None,
        };
        self.pending.push_back(job_id.clone());
        self.order.push_back(job_id.clone());
        self.entries.insert(
            job_id,
            Entry {
                request,
                receipt: receipt.clone(),
            },
        );
        Ok(receipt)
    }

    /// Dequeue one request for the real main-thread host implementation.
    pub fn take_pending(&mut self) -> Option<FeedbackRequest> {
        while let Some(id) = self.pending.pop_front() {
            let entry = self.entries.get(&id)?;
            if entry.receipt.status == FeedbackStatus::Queued {
                return Some(entry.request.clone());
            }
        }
        None
    }

    pub fn get(&self, job_id: &str) -> Option<&FeedbackReceipt> {
        self.entries.get(job_id).map(|entry| &entry.receipt)
    }
    pub fn list(&self) -> Vec<FeedbackReceipt> {
        self.order
            .iter()
            .filter_map(|id| self.get(id).cloned())
            .collect()
    }

    fn receipt_mut(&mut self, job_id: &str) -> Result<&mut FeedbackReceipt, String> {
        self.entries
            .get_mut(job_id)
            .map(|entry| &mut entry.receipt)
            .ok_or_else(|| "unknown or expired feedback job; verify session_id".into())
    }

    pub fn start(&mut self, job_id: &str) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if receipt.status != FeedbackStatus::Queued {
            return Err("only queued work can start".into());
        }
        receipt.status = FeedbackStatus::Running;
        Ok(())
    }

    pub fn progress(&mut self, job_id: &str, value: f32) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if receipt.status.terminal()
            || !value.is_finite()
            || !(0.0..=1.0).contains(&value)
            || value < receipt.progress
        {
            return Err("progress must be finite, monotonic and belong to active work".into());
        }
        receipt.progress = value;
        Ok(())
    }

    pub fn awaiting_publish(&mut self, job_id: &str) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if receipt.status != FeedbackStatus::Running {
            return Err("only running work can await publication".into());
        }
        receipt.status = FeedbackStatus::AwaitingPublish;
        Ok(())
    }

    /// Called only after real main-thread publication / GPU readback. A capture
    /// must identify the requested authoring revision; newer is not equivalent.
    pub fn succeed(&mut self, job_id: &str, revision: u64, result: Value) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if !matches!(
            receipt.status,
            FeedbackStatus::Running | FeedbackStatus::AwaitingPublish
        ) {
            return Err("only running work can complete".into());
        }
        if receipt.kind == FeedbackKind::Capture && revision != receipt.expected_revision {
            return Err("capture revision differs from the requested revision".into());
        }
        receipt.status = FeedbackStatus::Succeeded;
        receipt.progress = 1.0;
        receipt.published_revision = Some(revision);
        receipt.result = Some(result);
        Ok(())
    }

    pub fn fail(&mut self, job_id: &str, error: impl Into<String>) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if receipt.status.terminal() {
            return Err("terminal receipts cannot change".into());
        }
        receipt.status = FeedbackStatus::Failed;
        receipt.error = Some(error.into());
        Ok(())
    }

    /// Queued work cancels immediately. Running cancellation is only a request;
    /// the host forwards it to the real job and acknowledges when it has stopped.
    pub fn cancel(&mut self, job_id: &str) -> Result<FeedbackReceipt, String> {
        let receipt = self.receipt_mut(job_id)?;
        if !receipt.status.terminal() {
            receipt.cancellation_requested = true;
            if receipt.status == FeedbackStatus::Queued {
                receipt.status = FeedbackStatus::Cancelled;
            }
        }
        Ok(receipt.clone())
    }

    pub fn acknowledge_cancelled(&mut self, job_id: &str) -> Result<(), String> {
        let receipt = self.receipt_mut(job_id)?;
        if receipt.status.terminal() || !receipt.cancellation_requested {
            return Err("no active cancellation request".into());
        }
        receipt.status = FeedbackStatus::Cancelled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retry_and_cancel_never_publish_a_fake_success() {
        let mut queue = FeedbackQueue::new("session");
        let receipt = queue
            .submit("one", FeedbackKind::Import, json!({"path":"a.glb"}), 4)
            .unwrap();
        assert_eq!(
            queue
                .submit("one", FeedbackKind::Import, json!({"path":"a.glb"}), 4)
                .unwrap(),
            receipt
        );
        assert!(
            queue
                .submit("one", FeedbackKind::Import, json!({"path":"b.glb"}), 4)
                .is_err()
        );
        queue.start(&receipt.job_id).unwrap();
        assert_eq!(
            queue.cancel(&receipt.job_id).unwrap().status,
            FeedbackStatus::Running
        );
        queue.acknowledge_cancelled(&receipt.job_id).unwrap();
        assert!(queue.succeed(&receipt.job_id, 5, json!({})).is_err());
    }
    #[test]
    fn capture_completion_requires_the_exact_requested_revision() {
        let mut queue = FeedbackQueue::new("session");
        let receipt = queue
            .submit("capture", FeedbackKind::Capture, json!({}), 4)
            .unwrap();
        assert!(queue.succeed(&receipt.job_id, 4, json!({})).is_err());
        queue.start(&receipt.job_id).unwrap();
        assert!(queue.succeed(&receipt.job_id, 5, json!({})).is_err());
        queue
            .succeed(&receipt.job_id, 4, json!({"capture":{"frame":12}}))
            .unwrap();
        assert_eq!(
            queue.get(&receipt.job_id).unwrap().published_revision,
            Some(4)
        );
        assert!(queue.fail(&receipt.job_id, "late error").is_err());
    }
}
