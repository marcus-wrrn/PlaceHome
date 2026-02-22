use std::collections::HashMap;
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn, error};

use crate::services::ServiceId;

/// Current status of a managed service
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    /// Binary not found on system
    Unavailable,
    /// Binary found but service not started
    Stopped,
    /// Service is starting up
    Starting,
    /// Service is running
    Running { pid: u32 },
    /// Service failed to start or crashed
    Failed { reason: String },
}

/// Commands sent to the supervisor via channel
pub enum SupervisorCommand {
    Start {
        service: ServiceId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stop {
        service: ServiceId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Restart {
        service: ServiceId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Get status of all services
    Status {
        reply: oneshot::Sender<HashMap<ServiceId, ServiceStatus>>,
    },
}

/// Handle for sending commands to the supervisor
#[derive(Clone)]
pub struct SupervisorHandle {
    tx: mpsc::Sender<SupervisorCommand>,
}

impl SupervisorHandle {
    pub async fn start_service(&self, service: ServiceId) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Start { service, reply })
            .await
            .map_err(|_| "supervisor channel closed".to_string())?;
        rx.await.map_err(|_| "supervisor dropped reply".to_string())?
    }

    pub async fn stop_service(&self, service: ServiceId) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Stop { service, reply })
            .await
            .map_err(|_| "supervisor channel closed".to_string())?;
        rx.await.map_err(|_| "supervisor dropped reply".to_string())?
    }

    pub async fn restart_service(&self, service: ServiceId) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Restart { service, reply })
            .await
            .map_err(|_| "supervisor channel closed".to_string())?;
        rx.await.map_err(|_| "supervisor dropped reply".to_string())?
    }

    pub async fn get_status(&self) -> Result<HashMap<ServiceId, ServiceStatus>, String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Status { reply })
            .await
            .map_err(|_| "supervisor channel closed".to_string())?;
        rx.await.map_err(|_| "supervisor dropped reply".to_string())
    }
}

/// Trait for a managed child process
#[async_trait]
pub trait ManagedService {
    /// Spawn the service, returns its PID
    async fn start(&mut self) -> Result<u32, String>;
    /// Stop the service safely
    async fn stop(&mut self) -> Result<(), String>;
    async fn is_running(&mut self) -> bool;
}

/// The supervisor owns all managed services and processes commands sequentially
pub struct Supervisor {
    services: HashMap<ServiceId, Box<dyn ManagedService + Send>>,
    statuses: HashMap<ServiceId, ServiceStatus>,
    channel_buffer: usize,  // number of messages queued in service queue before drops
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
            statuses: HashMap::new(),
            channel_buffer: 32
        }
    }

    /// Register a service with the supervisor.
    /// If `available` is false, the service is marked Unavailable and won't be startable.
    pub fn register(
        &mut self,
        id: ServiceId,
        service: Box<dyn ManagedService + Send>,
        available: bool,
    ) {
        let initial_status = if available {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::Unavailable
        };
        self.statuses.insert(id.clone(), initial_status);
        self.services.insert(id, service);
    }

    /// Spawn the supervisor loop, returning a handle for sending commands.
    pub fn spawn(mut self) -> SupervisorHandle {
        let (tx, mut rx) = mpsc::channel::<SupervisorCommand>(self.channel_buffer);

        tokio::spawn(async move {
            info!("Supervisor started");

            while let Some(cmd) = rx.recv().await {
                match cmd {
                    SupervisorCommand::Start { service, reply } => {
                        let result = self.handle_start(&service).await;
                        let _ = reply.send(result);
                    }
                    SupervisorCommand::Stop { service, reply } => {
                        let result = self.handle_stop(&service).await;
                        let _ = reply.send(result);
                    }
                    SupervisorCommand::Restart { service, reply } => {
                        let result = self.handle_restart(&service).await;
                        let _ = reply.send(result);
                    }
                    SupervisorCommand::Status { reply } => {
                        let _ = reply.send(self.statuses.clone());
                    }
                }
            }

            info!("Supervisor shutting down, stopping all services");
            for (id, svc) in self.services.iter_mut() {
                if let Err(e) = svc.stop().await {
                    warn!("Failed to stop {:?} during shutdown: {}", id, e);
                }
            }
        });

        SupervisorHandle { tx }
    }

    async fn handle_start(&mut self, id: &ServiceId) -> Result<(), String> {
        match self.statuses.get(id) {
            Some(ServiceStatus::Unavailable) => {
                return Err(format!("{:?} is not installed on this system", id));
            }
            Some(ServiceStatus::Running { .. }) => {
                return Err(format!("{:?} is already running", id));
            }
            Some(ServiceStatus::Starting) => {
                return Err(format!("{:?} is already starting", id));
            }
            None => {
                return Err(format!("{:?} is not registered", id));
            }
            _ => {}
        }

        self.statuses.insert(id.clone(), ServiceStatus::Starting);

        let svc = self.services.get_mut(id)
            .ok_or_else(|| format!("{:?} not registered", id))?;

        match svc.start().await {
            Ok(pid) => {
                info!("{:?} started with PID {}", id, pid);
                self.statuses.insert(id.clone(), ServiceStatus::Running { pid });
                Ok(())
            }
            Err(e) => {
                error!("{:?} failed to start: {}", id, e);
                self.statuses.insert(id.clone(), ServiceStatus::Failed { reason: e.clone() });
                Err(e)
            }
        }
    }

    async fn handle_stop(&mut self, id: &ServiceId) -> Result<(), String> {
        match self.statuses.get(id) {
            Some(ServiceStatus::Running { .. }) | Some(ServiceStatus::Starting) => {}
            Some(ServiceStatus::Stopped) => {
                return Err(format!("{:?} is already stopped", id));
            }
            Some(ServiceStatus::Unavailable) => {
                return Err(format!("{:?} is not available", id));
            }
            _ => {}
        }

        let svc = self.services.get_mut(id)
            .ok_or_else(|| format!("{:?} not registered", id))?;

        match svc.stop().await {
            Ok(()) => {
                info!("{:?} stopped", id);
                self.statuses.insert(id.clone(), ServiceStatus::Stopped);
                Ok(())
            }
            Err(e) => {
                error!("{:?} failed to stop: {}", id, e);
                Err(e)
            }
        }
    }

    async fn handle_restart(&mut self, id: &ServiceId) -> Result<(), String> {
        // Stop if running (ignore errors if already stopped)
        if matches!(self.statuses.get(id), Some(ServiceStatus::Running { .. })) {
            let _ = self.handle_stop(id).await;
        }
        self.handle_start(id).await
    }
}
