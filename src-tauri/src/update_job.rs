use serde::Serialize;
use std::future::Future;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
pub struct UpdateStatus {
    pub revision: u64,
    pub phase: String,
    pub message: String,
    pub running: bool,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: "idle".to_string(),
            message: String::new(),
            running: false,
        }
    }
}

pub struct UpdateCoordinator {
    gate: tokio::sync::Mutex<()>,
    status: Mutex<UpdateStatus>,
}

impl UpdateCoordinator {
    pub fn new() -> Self {
        Self {
            gate: tokio::sync::Mutex::new(()),
            status: Mutex::new(UpdateStatus::default()),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set_status(
        &self,
        phase: &str,
        message: impl Into<String>,
        running: bool,
    ) -> UpdateStatus {
        let mut current = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next = UpdateStatus {
            revision: current.revision + 1,
            phase: phase.to_string(),
            message: message.into(),
            running,
        };
        *current = next.clone();
        next
    }

    pub async fn run<F, T>(&self, operation: F) -> Result<T, &'static str>
    where
        F: Future<Output = T>,
    {
        let _guard = self
            .gate
            .try_lock()
            .map_err(|_| "업데이트 작업이 이미 진행 중입니다")?;
        Ok(operation.await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn simultaneous_requests_run_installer_once() {
        let coordinator = std::sync::Arc::new(UpdateCoordinator::new());
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let first_coordinator = coordinator.clone();
        let first_count = count.clone();
        let first = tokio::spawn(async move {
            first_coordinator
                .run(async move {
                    first_count.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    finish_rx.await.unwrap();
                })
                .await
        });
        started_rx.await.unwrap();
        assert!(coordinator
            .run(async {
                count.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .is_err());
        finish_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
