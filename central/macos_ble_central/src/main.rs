use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use futures::stream::StreamExt;
use serde_json::Value;
use std::error::Error;
use std::io;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    // 1. Parse the UUIDs (must match your Android app)
    let service_uuid = Uuid::parse_str("9835D696-923D-44CA-A5EA-D252AE3297B9")?;
    let char_uuid    = Uuid::parse_str("7AB61943-BBB5-49D6-88C8-96185A98E587")?;

    // 2. Grab the first Bluetooth adapter
    let manager  = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central  = adapters.into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No Bluetooth adapter found"))?;

    // 3. Start scanning *only* for devices advertising our service
    let filter = ScanFilter { services: vec![service_uuid], ..Default::default() };
    central.start_scan(filter).await?;
    println!("Scanning for BLE peripheral…");

    // 4. Wait (with timeout) until we find it
    let mut attempts = 0;
    let peripheral = 'search: loop {
        if attempts >= 30 {
            return Err(Box::new(io::Error::new(
                io::ErrorKind::TimedOut,
                "Timed out waiting for peripheral",
            )));
        }
        attempts += 1;

        let list = central.peripherals().await?;
        for p in &list {
            if let Ok(Some(props)) = p.properties().await {
                if props.services.contains(&service_uuid) {
                    println!("Found peripheral: {:?}", p.address());
                    break 'search p.clone();
                }
            }
        }

        sleep(Duration::from_secs(1)).await;
    };

    // 5. Stop scanning now that we’ve got it
    central.stop_scan().await?;
    println!("Stopped scanning.");

    // 6. Connect & discover
    peripheral.connect().await?;
    println!("Connected to {:?}", peripheral.address());
    peripheral.discover_services().await?;
    println!("Services discovered.");

    // 7. Locate the characteristic we’re interested in
    let chars = peripheral.characteristics();
    let tx_char = chars.iter()
        .find(|c| c.uuid == char_uuid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Characteristic not found"))?
        .clone();

    // 8. Subscribe to notifications
    let mut notifications = peripheral.notifications().await?;
    println!("Attempting to subscribe…");
    match peripheral.subscribe(&tx_char).await {
        Ok(()) => println!("→ subscribe() returned OK"),
        Err(e) => eprintln!("→ subscribe() ERROR: {}", e),
    }
    println!("Subscribed to notifications on {}", char_uuid);

    // 9. Read & parse JSON as it comes in
    println!("Listening for JSON notifications…");
    while let Some(data) = notifications.next().await {
        if data.uuid == char_uuid {
            match String::from_utf8(data.value.clone()) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(json) => println!("→ {}", json),
                    Err(e)   => eprintln!("Invalid JSON: {} (raw={:?})", e, data.value),
                },
                Err(_) => eprintln!("Non-UTF8 data: {:?}", data.value),
            }
        }
    }

    // 10. Clean up
    println!("Notification stream ended; cleaning up…");
    peripheral.unsubscribe(&tx_char).await?;
    println!("Unsubscribed.");
    peripheral.disconnect().await?;
    println!("Disconnected.");

    Ok(())
}
