use super::CaService;

/// Construct and initialise the CA service.
///
/// Returns an initialised [`CaService`] with the root CA loaded into memory,
/// or an error string if generation/loading fails.
pub async fn register() -> Result<CaService, String> {
    let service = CaService::new();
    service.init().await?;
    Ok(service)
}
