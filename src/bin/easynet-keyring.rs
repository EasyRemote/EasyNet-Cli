// EasyNet Key Service — process entry point
// ==========================================
//
// Bounded transport, protocol dispatch, and encrypted custody are one
// cohesive library-owned service. The daemon lifecycle manager owns process
// supervision; this binary only establishes the custody process boundary.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    easynet_cli::daemon::keyring::service::run_default_key_service().await
}
