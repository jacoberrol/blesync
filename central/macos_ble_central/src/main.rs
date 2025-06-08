use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use serde_json::Value;
use std::error::Error;
use tokio::{signal};
use tokio::time::{sleep, Duration};
use uuid::Uuid;
use log::{error, warn, info, debug, /*trace*/};

/// A single object that owns our BLE state.
struct BleCentral {
    // manager: Manager,            // the btleplug Manager
    adapter: Adapter,               // the selected BLE adapter
    peripheral: Option<Peripheral>, // the connected peripheral, once found
    service_uuid: Uuid,
    char_uuid: Uuid,
}

impl BleCentral {
    
    /// Construct and initialize logging + BLE manager + adapter
    async fn new(service: &str, characteristic: &str) -> Result<Self, Box<dyn Error>> {
 
        /*
        * Step 1: Parse the UUIDs
        * - We define 128-bit UUIDs for the BLE service and characteristic.
        * - Uuid::parse_str parses a hyphenated string into a Uuid instance.
        * - These must match the peripheral (Android) side exactly.
        */
        let service_uuid = Uuid::parse_str(service)?;
        let char_uuid    = Uuid::parse_str(characteristic)?;
 
        /*
        * Step 2: Acquire the Bluetooth adapter via btleplug
        * - Manager::new() initializes the BLE manager implementation (CoreBluetooth on macOS).
        * - manager.adapters() returns available adapters (e.g., built-in, USB dongles).
        * - We take the first adapter; error if none found.
        */
        let manager  = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter  = adapters.into_iter()
            .next()
            .ok_or("No Bluetooth adapter found")?;
 
        Ok(Self {
            // manager,
            adapter,
            peripheral: None,
            service_uuid,
            char_uuid,
        })
    }


    /// Scan until we find the peripheral, then store it in self.peripheral
    async fn scan_and_select(&mut self) -> Result<(), Box<dyn Error>> {
        /*
        * Step 3: Start scanning for peripherals advertising our service UUID
        * - ScanFilter configures the BLE library to only return advertisements containing our service.
        * - adapter.start_scan triggers the OS BLE scan.
        */
        let filter = ScanFilter { services: vec![self.service_uuid], ..Default::default() };
        self.adapter.start_scan(filter).await?;
        info!("Scanning for BLE peripheral…");

        /*
        * Step 4: Poll until we discover our target peripheral (with timeout)
        * - Loop with a cap on attempts (30 seconds max).
        * - adapter.peripherals() lists discovered devices so far.
        * - p.properties().await fetches advertisement metadata including services.
        * - We compare the advertised services list to our target UUID.
        */
        for _ in 0..30 {
            let list = self.adapter.peripherals().await?;
            for p in &list {
                if let Ok(Some(props)) = p.properties().await {
                    if props.services.contains(&self.service_uuid) {
                        info!("Found peripheral {}", p.address());
                        self.peripheral = Some(p.clone());
                        self.adapter.stop_scan().await?;
                        debug!("Stopped scanning.");
                        return Ok(());
                    }
                }
            }
            sleep(Duration::from_secs(1)).await;
        }
        Err("Timed out".into())

    }

    /// Connect, discover, subscribe, and process notifications
    async fn run_session(&mut self) -> Result<(), Box<dyn Error>> {

        let periph = self.peripheral.as_ref().ok_or("No peripheral selected")?;

        let sig = signal::ctrl_c();
        tokio::pin!(sig);

        /*
        * Step 5: Connect to the peripheral and discover its services
        * - peripheral.connect() establishes a GATT connection.
        * - peripheral.discover_services() populates the GATT service and characteristic cache.
        */
        periph.connect().await?;
        info!("Connected to {:?}", periph.address());
        periph.discover_services().await?;
        debug!("Services discovered");

        /*
        * Step 6: Locate the specific GATT characteristic to subscribe to
        * - peripheral.characteristics() returns a Vec of all characteristics.
        * - We find the one matching our UUID and clone it for use.
        */
        let chars = periph.characteristics();
        let tx_char = chars.iter()
            .find(|c| c.uuid == self.char_uuid)
            .ok_or("Characteristic not found")?
            .clone();

        /*
        * Step 7: Subscribe to notifications on that characteristic
        * - peripheral.notifications() yields a stream of incoming notifications.
        * - peripheral.subscribe() writes to the CCCD descriptor to enable notifications.
        */
        let mut notifications = periph.notifications().await?;
        debug!("Attempting to subscribe…");
        match periph.subscribe(&tx_char).await {
            Ok(()) => debug!("→ subscribe() returned OK"),
            Err(e) => error!("→ subscribe() ERROR: {}", e),
        }
        info!("Subscribed to notifications on {}", self.char_uuid);


        /*
        * Step 8: Process incoming notification packets
        * - We loop on notifications.next() which awaits the next notification.
        * - Each notification has a UUID and raw byte Vec payload.
        * - We convert it to UTF-8, then parse as JSON using serde_json.
        */
        debug!("Listening for JSON notifications…");
        loop {
            tokio::select! {
                notif = notifications.next() => {
                    match notif {
                        Some(n) => {
                            if n.uuid == self.char_uuid {
                                let text = String::from_utf8_lossy(&n.value);
                                match serde_json::from_str::<Value>(&text) {
                                    Ok(json) => info!("→ {}", json),
                                    Err(e)   => error!("JSON parse error: {}", e),
                                }
                            }
                        },
                        None => break,
                    }
                }

                _ = &mut sig => {
                    warn!("Shutdown signal recieved!");
                    break;
                }

                _ = sleep(Duration::from_secs(10)) => {
                    match periph.is_connected().await {
                        Ok(true) => {
                            continue;
                        }
                        Ok(false) => {
                            warn!("Detected peripheral disconnect.");
                            break;
                        }
                        Err(e) => {
                            error!("Error checking connection state: {}",e);
                            break;
                        }
                    }
                }
           }
        }

        /*
        * Step 9: Cleanup GATT subscription and disconnect
        * - peripheral.unsubscribe() disables notifications on the CCCD.
        * - peripheral.disconnect() tears down the GATT connection.
        */
        info!("Notification stream ended; cleaning up…");
        periph.unsubscribe(&tx_char).await?;
        debug!("Unsubscribed.");
        periph.disconnect().await?;
        debug!("Disconnected.");
        Ok(())

    }


    /// High‐level reconnect loop
    async fn run(mut self) {
        loop {
            // 1) Scan & select
            if let Err(e) = self.scan_and_select().await {
                warn!("Scan failed: {} — retrying in 5s…", e);
            }
            // 2) Only if scan succeeded, run the session
            else if let Err(e) = self.run_session().await {
                warn!("Session error: {} — retrying in 5s…", e);
            }
            // 3) Back off before trying again
            sleep(Duration::from_secs(5)).await;
        }
    }

}


#[tokio::main]
async fn main() {

    // initialize logger with default level "info"
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();
    
    // Create the single owner for all BLE state
    let ble = BleCentral::new(
        "9835D696-923D-44CA-A5EA-D252AE3297B9",
        "7AB61943-BBB5-49D6-88C8-96185A98E587"
    ).await.expect("Initialization failed");
    
    // Enter the automatic reconnect loop
    ble.run().await;
}