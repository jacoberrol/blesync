use crate::error::BleError;
use btleplug::api::{Central, Characteristic, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use serde_json::Value;
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;
use tracing::{debug, error, info, instrument, warn /*trace*/};


#[derive(Debug)]
pub struct BleConfig {
    pub scan_retries:     u32,
    pub scan_interval:    Duration,
    pub notify_timeout:   Duration,
    pub reconnect_backoff: Duration,
}

impl Default for BleConfig {
    fn default() -> Self {
        BleConfig {
            scan_retries:      30,
            scan_interval:     Duration::from_secs(1),
            notify_timeout:    Duration::from_secs(10),
            reconnect_backoff: Duration::from_secs(5),
        }
    }
}

pub struct BleCentral {
    manager: Option<Manager>,               // the BLE Manager, once instantiated
    adapter: Option<Adapter>,               // the BLE adapter, once instantiated
    peripheral: Option<Peripheral>,         // the connected peripheral, once found
    characteristic: Option<Characteristic>, // the discovered characteristic, once found
    service_uuid: Uuid,                     // the service UUID to discover
    char_uuid: Uuid,                        // the characteristic UUID to discover
    config: BleConfig,
}

impl BleCentral {
    
    /// Construct and initialize logging + BLE manager + adapter
    pub async fn new(service: &str, characteristic: &str, config: Option<BleConfig>) -> Result<Self, BleError> {
        info!("Constructing BLE Central.");
        /*
        * Step 1: Parse the UUIDs
        * - We define 128-bit UUIDs for the BLE service and characteristic.
        * - Uuid::parse_str parses a hyphenated string into a Uuid instance.
        * - These must match the peripheral (Android) side exactly.
        */
        Ok(Self {
            manager: None,
            adapter: None,
            peripheral: None,
            characteristic: None,
            service_uuid: Uuid::parse_str(service)?,
            char_uuid: Uuid::parse_str(characteristic)?,
            config: config.unwrap_or_default(),
        })
    }

    /// recreate my bluetooth adapater and store in self.adapter
    #[instrument(skip(self))]    
    async fn recreate_adapter(&mut self) -> Result<(), BleError> {
        info!("Recreating BLE Adapter.");
        /*
        * Step 2: Acquire the Bluetooth adapter via btleplug
        * - Manager::new() initializes the BLE manager implementation (CoreBluetooth on macOS).
        * - manager.adapters() returns available adapters (e.g., built-in, USB dongles).
        * - We take the first adapter; error if none found.
        */
        let manager  = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter  = adapters
            .into_iter()
            .next()
            .ok_or(BleError::NoAdapter)?;
        self.manager = Some(manager);
        self.adapter = Some(adapter);
        Ok(())
    }

    /// Scan until we find the peripheral, then store it in self.peripheral
    #[instrument(skip(self))]
    async fn scan_and_select(&mut self) -> Result<(), BleError> {
        info!("Scanning for peripheral.");

        let adapt = self.adapter
            .as_ref()
            .ok_or(BleError::NoAdapter)?;

        /*
        * Step 3: Start scanning for peripherals advertising our service UUID
        * - ScanFilter configures the BLE library to only return advertisements containing our service.
        * - adapter.start_scan triggers the OS BLE scan.
        */
        let filter = ScanFilter { services: vec![self.service_uuid], ..Default::default() };        
        
        adapt.start_scan(filter).await?;
        debug!("Started Scanning for BLE peripheral…");

        /*
        * Step 4: Poll until we discover our target peripheral (with timeout)
        * - Loop with a cap on attempts (30 seconds max).
        * - adapter.peripherals() lists discovered devices so far.
        * - p.properties().await fetches advertisement metadata including services.
        * - We compare the advertised services list to our target UUID.
        */
        'scan: for _ in 0..self.config.scan_retries {
            let list = adapt.peripherals().await?;
            for p in &list {
                // Perform the async properties() call outside of a closure
                if let Ok(Some(props)) = p.properties().await {
                    if props.services.contains(&self.service_uuid) {
                        self.peripheral = Some(p.clone());
                        info!("Found peripheral {}", p.address());
                        break 'scan;
                    }
                }
            }
            // sleep for 1s before trying again
            debug!("no peripheral found. sleep and retry");
            sleep(self.config.scan_interval).await;
        }

        adapt.stop_scan().await?;
        debug!("Stopped scanning.");

        self.peripheral
            .as_ref()
            .ok_or(BleError::NoPeripheral)?;
        
        Ok(())

    }

    /// connect to the peripheral and discover its services
    #[instrument(skip(self))]
    async fn connect_and_discover(&mut self) -> Result<(), BleError> {
        info!("Connecting to peripheral and discovering services.");
        
        let periph = self.peripheral
            .as_ref()
            .ok_or(BleError::NoPeripheral)
            .unwrap();

        /*
        * Step 5: Connect to the peripheral and discover its services
        * - peripheral.connect() establishes a GATT connection.
        * - peripheral.discover_services() populates the GATT service and characteristic cache.
        */
        periph.connect().await?;
        debug!("Connected to {:?}", periph.address());
        periph.discover_services().await?;
        debug!("Services discovered");

        /*
        * Step 6: Locate the specific GATT characteristic to subscribe to
        * - peripheral.characteristics() returns a Vec of all characteristics.
        * - We find the one matching our UUID and clone it for use.
        */
        let chars = periph.characteristics();
        self.characteristic = chars.iter()
            .find(|c| c.uuid == self.char_uuid)
            .cloned();

        Ok(())

    }

    /// Connect, discover, subscribe, and process notifications
    #[instrument(skip(self))]
    async fn run_session(&mut self) -> Result<(), BleError> {
        info!("Starting session.");

        // proceed only if we have a reference to the peripheral
        let periph = self.peripheral
            .as_ref()
            .ok_or(BleError::NoPeripheral)?;

        let tx_char = self.characteristic
            .as_ref()
            .ok_or(BleError::NoCharacteristic(self.char_uuid))?;

        /*
        * Step 7: Subscribe to notifications on that characteristic
        * - peripheral.notifications() yields a stream of incoming notifications.
        * - peripheral.subscribe() writes to the CCCD descriptor to enable notifications.
        */
        let mut notifications = periph.notifications().await?; 
        debug!("Attempting to subscribe…");
        periph.subscribe(tx_char).await?;
        info!("Subscribed to notifications on {}", self.char_uuid);

        /*
        * Step 8: Process incoming notification packets
        * - We loop on notifications.next() which awaits the next notification.
        * - Each notification has a UUID and raw byte Vec payload.
        * - We convert it to UTF-8, then parse as JSON using serde_json.
        */
        debug!("Listening for JSON notifications…");
        loop {
            match timeout(self.config.notify_timeout,notifications.next()).await {
                Ok(Some(n)) => {
                    if n.uuid == self.char_uuid {
                        let text = String::from_utf8_lossy(&n.value);
                        match serde_json::from_str::<Value>(&text) {
                            Ok(json) => info!("→ {}", json),
                            Err(e)   => error!("JSON parse error: {}", e),
                        }
                    }
                },
                _ => {
                    // covers Ok(None) (stream ended), Err(_) (timeout), or any error
                    warn!("Notifications stopped or timed out; disconnecting");
                    return Err(BleError::SessionEnded);
                },
            }
        }
    }

    /// High‐level reconnect loop
    #[instrument(skip(self))]
    pub async fn run(&mut self) {
        loop {
            // 1) Recreate Adapter
            if let Err(e) = self.recreate_adapter().await {
                warn!("Failed to get adapter: {} — retrying in 5s", e);
                sleep(self.config.reconnect_backoff).await;
                continue;
            }
            // 2) Scan & select
            if let Err(e) = self.scan_and_select().await {
                warn!("Scan failed: {} — retrying in 5s…", e);
                sleep(self.config.reconnect_backoff).await;
                continue;
            }
            // 3) Connect & discover
            if let Err(e) = self.connect_and_discover().await {
                warn!("Discover failed: {} — retrying in 5s…", e);
                sleep(self.config.reconnect_backoff).await;
                continue;
            }
            // 4) Run session
            if let Err(e) = self.run_session().await {
                warn!("Session error: {} — retrying in 5s…", e);
                // drop old peripheral & characteristic
                self.peripheral = None;
                self.characteristic = None;
                sleep(self.config.reconnect_backoff).await;
                continue;
            }
            // if run_session() ever returns Ok, we exit the loop
            info!("Session ended normally; exiting");
            break;
        }
    }

    /// shutdown function
    #[instrument(skip(self))]
    pub async fn shutdown(&mut self) {
        info!("Shutting down BLE.");
        if let Some(per) = &self.peripheral {
            if let Some(tx_char) = &self.characteristic {
                let _ = per.unsubscribe(tx_char);
                debug!("Unsubscribed");
            }
            let _ = per.disconnect();
            debug!("Disconnected");
            debug!("Performed shutdown cleanup");
        }
    }

}