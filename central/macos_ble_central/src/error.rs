#[derive(Debug, thiserror::Error)]
pub enum BleError {
    #[error("UUID parsing failed: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("No Bluetooth adapter found")]
    NoAdapter,

    #[error("Scan timed out after {0} seconds")]
    ScanTimeout(u64),

    #[error("Peripheral not found")]
    NoPeripheral,

    #[error("Characteristic not found: {0}")]
    NoCharacteristic(uuid::Uuid),

    #[error("BLE operation failed: {0}")]
    Api(#[from] btleplug::Error),

    #[error("Session ended (peripheral disconnected or adapter lost)")]
    SessionEnded,
}