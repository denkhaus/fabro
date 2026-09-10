//! The sandbox driver's events for a run's sandbox, as workflow events.
//!
//! A run's sandbox is created or attached with a driver [`EventContext`]
//! whose observer is a [`SandboxEventBridge`]. The driver reports every
//! operation it performs — start, stop, delete, the image pull inside a
//! create, snapshot builds — and the bridge turns the ones fabro records
//! on a run into [`SandboxLifecycle`] events. Everything else the driver
//! reports (state observations, notices, other operations) is not a run
//! event and is dropped here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sandbox_driver::{
    Action, ErrorReport, Event as DriverEvent, EventBody as DriverEventBody, EventObserver,
    EventSubject, OperationId, ProgressCode,
};

use super::{Emitter, Event, SandboxLifecycle};

/// Emits the workflow's sandbox lifecycle events from the driver's.
pub struct SandboxEventBridge {
    emitter:  Arc<Emitter>,
    /// Fabro's name for the provider, which is what the run records; the
    /// driver's own kind name can differ (`host` for a `local` run).
    provider: String,
    /// The image the sandbox is created from, named on pull events.
    image:    Option<String>,
    /// Creates that pulled an image, by operation, with when the pull began.
    pulls:    Mutex<HashMap<OperationId, Instant>>,
}

impl SandboxEventBridge {
    pub fn new(emitter: Arc<Emitter>, provider: impl Into<String>, image: Option<String>) -> Self {
        Self {
            emitter,
            provider: provider.into(),
            image,
            pulls: Mutex::new(HashMap::new()),
        }
    }

    /// The lifecycle event a driver event stands for, if fabro records one.
    fn translate(&self, event: &DriverEvent) -> Option<SandboxLifecycle> {
        match &event.subject {
            EventSubject::Sandbox { .. } => self.translate_sandbox(event),
            EventSubject::Snapshot { id, name } => {
                let name = name
                    .clone()
                    .or_else(|| id.as_ref().map(ToString::to_string))
                    .unwrap_or_default();
                match &event.body {
                    DriverEventBody::OperationStarted { .. } => {
                        Some(SandboxLifecycle::SnapshotCreating { name })
                    }
                    DriverEventBody::OperationCompleted { duration, .. } => {
                        Some(SandboxLifecycle::SnapshotReady {
                            name,
                            duration_ms: duration_ms(*duration),
                        })
                    }
                    DriverEventBody::OperationFailed { error, .. } => {
                        Some(SandboxLifecycle::SnapshotFailed {
                            name,
                            error: error.message.clone(),
                            causes: error.causes.clone(),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn translate_sandbox(&self, event: &DriverEvent) -> Option<SandboxLifecycle> {
        let provider = self.provider.clone();
        match &event.body {
            DriverEventBody::OperationStarted { action } => match action {
                Action::Start => Some(SandboxLifecycle::StartStarted { provider }),
                Action::Stop => Some(SandboxLifecycle::StopStarted { provider }),
                Action::Delete => Some(SandboxLifecycle::DeleteStarted { provider }),
                _ => None,
            },
            DriverEventBody::OperationProgress { action, progress } => {
                if *action != Action::Create || progress.code.as_str() != ProgressCode::IMAGE_PULL {
                    return None;
                }
                // The first pull report of a create opens the pull; later
                // ones are the same pull's progress.
                let operation_id = event.operation_id.clone()?;
                let mut pulls = self.pulls.lock().unwrap_or_else(PoisonError::into_inner);
                if pulls.contains_key(&operation_id) {
                    return None;
                }
                pulls.insert(operation_id, Instant::now());
                Some(SandboxLifecycle::SnapshotPulling {
                    name: self
                        .image
                        .clone()
                        .or_else(|| progress.message.clone())
                        .unwrap_or_default(),
                })
            }
            DriverEventBody::OperationCompleted { action, duration } => match action {
                Action::Create => {
                    let pulled = self.take_pull(event.operation_id.as_ref())?;
                    Some(SandboxLifecycle::SnapshotReady {
                        name:        self.image.clone().unwrap_or_default(),
                        duration_ms: duration_ms(pulled.elapsed()),
                    })
                }
                Action::Start => Some(SandboxLifecycle::StartCompleted {
                    provider,
                    duration_ms: duration_ms(*duration),
                }),
                Action::Stop => Some(SandboxLifecycle::StopCompleted {
                    provider,
                    duration_ms: duration_ms(*duration),
                }),
                Action::Delete => Some(SandboxLifecycle::DeleteCompleted {
                    provider,
                    duration_ms: duration_ms(*duration),
                }),
                _ => None,
            },
            DriverEventBody::OperationFailed { action, error, .. } => match action {
                Action::Create => {
                    self.take_pull(event.operation_id.as_ref())?;
                    Some(SandboxLifecycle::SnapshotFailed {
                        name:   self.image.clone().unwrap_or_default(),
                        error:  error.message.clone(),
                        causes: error.causes.clone(),
                    })
                }
                Action::Start => Some(failed(error, |error, causes| {
                    SandboxLifecycle::StartFailed {
                        provider,
                        error,
                        causes,
                    }
                })),
                Action::Stop => Some(failed(error, |error, causes| {
                    SandboxLifecycle::StopFailed {
                        provider,
                        error,
                        causes,
                    }
                })),
                Action::Delete => Some(failed(error, |error, causes| {
                    SandboxLifecycle::DeleteFailed {
                        provider,
                        error,
                        causes,
                    }
                })),
                _ => None,
            },
            _ => None,
        }
    }

    /// When the create `operation_id` began pulling its image, if it did.
    fn take_pull(&self, operation_id: Option<&OperationId>) -> Option<Instant> {
        self.pulls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(operation_id?)
    }
}

fn failed(
    error: &ErrorReport,
    build: impl FnOnce(String, Vec<String>) -> SandboxLifecycle,
) -> SandboxLifecycle {
    build(error.message.clone(), error.causes.clone())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[async_trait]
impl EventObserver for SandboxEventBridge {
    async fn observe(&self, event: DriverEvent) {
        if let Some(lifecycle) = self.translate(&event) {
            self.emitter.emit(&Event::Sandbox { event: lifecycle });
        }
    }
}
