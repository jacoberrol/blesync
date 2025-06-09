use blesync::ble_central::BleCentral;
use tracing::{error, info};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
    async fn main() {

        // initialize logger with default level "debug"
        // env_logger::Builder::from_env(
        //     env_logger::Env::default().default_filter_or("trace")
        // ).init();

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("trace"));
        fmt().with_env_filter(filter).init();
        
        // Create the BleCentral
        let mut ble = BleCentral::new(
            "9835D696-923D-44CA-A5EA-D252AE3297B9",
            "7AB61943-BBB5-49D6-88C8-96185A98E587",
            None
        ).await.expect("Initialization failed");
        
        let shutdown = tokio::signal::ctrl_c();
        let runner  = ble.run();

        tokio::select! {
            _ = runner => {                
                error!("BLE loop exited unexpectedly; shutting down");
                ble.shutdown().await;
            }
            _ = shutdown => {
                info!("Ctrl-C received; shutting down");
                ble.shutdown().await;
            }
        }

    }