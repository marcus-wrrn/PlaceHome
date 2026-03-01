use tracing::{error, info};

use crate::config::HttpConfig;
use crate::supervisor::{Supervisor, SupervisorHandle};
use crate::services::ServiceId;
use super::HttpService;
use super::handshake::MqttBrokerageInfo;

/// Build and register the HTTP service onto the supervisor.
pub fn register_onto(supervisor: &mut Supervisor, config: HttpConfig, brokerage_info: MqttBrokerageInfo) {
    let service = HttpService::new(config, brokerage_info);
    supervisor.register(ServiceId::Http, Box::new(service), true);
}

/// Start the HTTP service via the supervisor.
pub async fn start_http(supervisor_handle: &SupervisorHandle) {
    match supervisor_handle.start_service(ServiceId::Http).await {
        Ok(()) => info!("HTTP server started"),
        Err(e) => error!("Failed to start HTTP service: {}", e),
    }
}
