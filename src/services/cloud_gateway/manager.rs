use tracing::{info, warn, error};

use crate::supervisor::{Supervisor, SupervisorHandle};
use crate::services::ServiceId;
use super::{CloudGatewayHandle, CloudGatewayService};

/// Build and register the cloud gateway client service.
///
/// Returns `None` when `gateway_url` is not configured — in that case the
/// service is registered as `Unavailable` and will not be started.
pub fn register_onto(
    supervisor: &mut Supervisor,
    server_url: String,
    gateway_url: Option<String>,
) -> Option<CloudGatewayHandle> {
    match gateway_url {
        None => {
            warn!("PLACENET_GATEWAY_URL not set — cloud gateway client disabled");
            None
        }
        Some(url) => {
            let service = CloudGatewayService::new(server_url, url);
            let handle = service.handle();
            supervisor.register(ServiceId::CloudGateway, Box::new(service), true);
            Some(handle)
        }
    }
}

/// Start the cloud gateway client via the supervisor.
pub async fn start_cloud_gateway(supervisor_handle: &SupervisorHandle) {
    match supervisor_handle.start_service(ServiceId::CloudGateway).await {
        Ok(()) => info!("Cloud gateway client started"),
        Err(e) => error!("Failed to start cloud gateway client: {}", e),
    }
}
