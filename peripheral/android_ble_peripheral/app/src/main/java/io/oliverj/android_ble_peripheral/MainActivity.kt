// File: app/src/main/java/io/oliverj/android_ble_peripheral/MainActivity.kt

package io.oliverj.android_ble_peripheral

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.bluetooth.*
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import java.util.UUID

// -----------------------------------------------------------------------------------------
// Logging tag for all BLE-peripheral related messages.
// REQUEST_PERMISSIONS: request code used when asking the user to grant runtime permissions.
// REQUIRED_PERMISSIONS: BLE on pre-API-31 Android requires BLUETOOTH, BLUETOOTH_ADMIN, and LOCATION.
// SERVICE_UUID / CHAR_UUID: 128-bit identifiers for our custom GATT service & characteristic.
// -----------------------------------------------------------------------------------------
private const val TAG = "BlePeripheral"
private const val REQUEST_PERMISSIONS = 1
private val REQUIRED_PERMISSIONS = arrayOf(
    Manifest.permission.BLUETOOTH,
    Manifest.permission.BLUETOOTH_ADMIN,
    Manifest.permission.ACCESS_FINE_LOCATION
)
private val SERVICE_UUID = UUID.fromString("9835D696-923D-44CA-A5EA-D252AE3297B9")
private val CHAR_UUID    = UUID.fromString("7AB61943-BBB5-49D6-88C8-96185A98E587")

class MainActivity : Activity() {

    // -------------------------------------------------------------------------------------
    // BluetoothLeAdvertiser: Android LE API for advertising BLE packets.
    // BluetoothGattServer: Android server API that hosts GATT services & handles client requests.
    // BluetoothGattCharacteristic: Represents a GATT characteristic (our data endpoint).
    // Handler / Looper: Android threading construct to schedule future tasks on the main thread.
    // connectedDevices: tracks which BLE centrals are currently connected for notifications.
    // lastValue: holds the most recent JSON payload bytes to respond to read requests.
    // -------------------------------------------------------------------------------------
    private lateinit var advertiser: BluetoothLeAdvertiser
    private lateinit var gattServer: BluetoothGattServer
    private lateinit var char: BluetoothGattCharacteristic
    private val handler = Handler(Looper.getMainLooper())
    private var counter = 0

    private val connectedDevices = mutableSetOf<BluetoothDevice>()
    private var lastValue: ByteArray = ByteArray(0)

    // -------------------------------------------------------------------------------------
    // onCreate(): Android entry point when the Activity is created.
    //   - Checks / requests runtime permissions required for BLE.
    //   - Only after permissions granted do we start our BLE peripheral logic.
    // -------------------------------------------------------------------------------------
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.d(TAG, "onCreate()")

        val ok = hasAllPermissions()
        Log.d(TAG, "Permissions granted? $ok")
        if (!ok) {
            Log.d(TAG, "Requesting permissions")
            ActivityCompat.requestPermissions(this, REQUIRED_PERMISSIONS, REQUEST_PERMISSIONS)
        } else {
            startBlePeripheral()
        }
    }

    // -------------------------------------------------------------------------------------
    // hasAllPermissions(): Utility to check each required BLE permission at runtime.
    //   - Uses ContextCompat.checkSelfPermission for each permission.
    // -------------------------------------------------------------------------------------
    private fun hasAllPermissions(): Boolean =
        REQUIRED_PERMISSIONS.all { perm ->
            ContextCompat.checkSelfPermission(this, perm) == PackageManager.PERMISSION_GRANTED
        }

    // -------------------------------------------------------------------------------------
    // onDestroy(): Clean up BLE resources when the Activity is destroyed.
    //   - Stop advertising to free the radio.
    //   - Close GATT server to free system resources.
    //   - Remove any pending Handler callbacks to avoid leaks.
    // -------------------------------------------------------------------------------------
    @SuppressLint("MissingPermission")
    override fun onDestroy() {
        super.onDestroy()
        Log.d(TAG, "onDestroy(): stopping advertising and closing GATT")
        advertiser.stopAdvertising(advertiseCallback)
        gattServer.close()
        handler.removeCallbacksAndMessages(null)
    }

    // -------------------------------------------------------------------------------------
    // onRequestPermissionsResult(): Called when the user responds to the permission prompt.
    //   - If granted, we proceed to start the BLE peripheral.
    //   - If denied, we log an error; no BLE actions will be possible.
    // -------------------------------------------------------------------------------------
    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        Log.d(TAG, "onRequestPermissionsResult(requestCode=$requestCode) results=${grantResults.joinToString()}")
        if (requestCode == REQUEST_PERMISSIONS &&
            grantResults.isNotEmpty() &&
            grantResults.all { it == PackageManager.PERMISSION_GRANTED }
        ) {
            Log.d(TAG, "Permissions granted by user, starting BLE peripheral")
            startBlePeripheral()
        } else {
            Log.e(TAG, "Permissions denied, cannot start BLE")
        }
    }

    // -------------------------------------------------------------------------------------
    // startBlePeripheral(): Sets up and starts the BLE GATT server & advertising.
    //   1) Obtain BluetoothManager & adapter -> entry points into Android’s BLE stack.
    //   2) Create & register a GATT characteristic for READ + NOTIFY.
    //   3) Add the standard Client Characteristic Configuration Descriptor (0x2902)
    //      so centrals can enable notifications.
    //   4) Create a primary GATT service, add our characteristic to it, register it.
    //   5) Build an AdvertiseData payload containing only our service UUID
    //      to stay under the 31-byte BLE advertising limit.
    //   6) Start advertising with low-latency, connectable settings.
    //   7) Schedule periodic `updateCharacteristic()` calls to push JSON updates.
    // -------------------------------------------------------------------------------------
    @SuppressLint("MissingPermission")
    private fun startBlePeripheral() {
        val btManager = getSystemService(BLUETOOTH_SERVICE) as BluetoothManager
        val adapter = btManager.adapter

        Log.d(TAG, "Adapter name=${adapter.name}, address=${adapter.address}")
        Log.d(TAG, "Supports peripheral mode? ${adapter.isMultipleAdvertisementSupported}")

        advertiser = adapter.bluetoothLeAdvertiser
        gattServer = btManager.openGattServer(this, gattServerCallback)

        // 1) Create the characteristic with READ + NOTIFY properties
        char = BluetoothGattCharacteristic(
            CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ
        )

        // 2) Add the CCCD descriptor (UUID 0x2902) so clients can enable/disable notifications
        val cccdUuid = UUID.fromString("00002902-0000-1000-8000-00805F9B34FB")
        val cccd = BluetoothGattDescriptor(
            cccdUuid,
            BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE
        )
        char.addDescriptor(cccd)
        Log.d(TAG, "Added CCCD descriptor: $cccdUuid")

        // 3) Create a primary GATT service, add the characteristic, and register with the GATT server
        val service = BluetoothGattService(
            SERVICE_UUID,
            BluetoothGattService.SERVICE_TYPE_PRIMARY
        )
        service.addCharacteristic(char)
        gattServer.addService(service)
        Log.d(TAG, "Service added: $SERVICE_UUID, Characteristic: $CHAR_UUID")

        // 4) Prepare the advertising payload: only the service UUID to minimize packet size
        val advertiseData = AdvertiseData.Builder()
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()

        // 5) Configure advertising settings: low latency & connectable
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setConnectable(true)
            .build()

        Log.d(TAG, "Starting advertising with data: $advertiseData")
        advertiser.startAdvertising(settings, advertiseData, advertiseCallback)

        // 6) Schedule the first JSON notification after 5 seconds
        handler.postDelayed(::updateCharacteristic, 5000)
    }

    // -------------------------------------------------------------------------------------
    // updateCharacteristic(): Updates the characteristic value and sends notifications.
    //   - Builds a simple JSON payload containing a timestamp & counter.
    //   - Stores the payload in `lastValue` for subsequent read requests.
    //   - Writes the bytes into the characteristic (deprecated API on Android < 33).
    //   - Iterates over connectedDevices, calling notifyCharacteristicChanged to push them.
    //   - Re-posts itself after 5s for continuous updates.
    // -------------------------------------------------------------------------------------
    @SuppressLint("MissingPermission")
    private fun updateCharacteristic() {
        val json = """{"timestamp":${System.currentTimeMillis()},"count":${counter++}}"""
        Log.d(TAG, "updateCharacteristic(): $json")
        val data = json.toByteArray(Charsets.UTF_8)
        lastValue = data

        @Suppress("DEPRECATION")
        char.value = data

        for (device in connectedDevices) {
            Log.d(TAG, "Notifying $device")
            @Suppress("DEPRECATION")
            gattServer.notifyCharacteristicChanged(device, char, /* confirm= */ false)
        }

        handler.postDelayed(::updateCharacteristic, 5000)
    }

    // -------------------------------------------------------------------------------------
    // advertiseCallback: Handles the result of startAdvertising().
    //   - onStartSuccess: BLE hardware confirmed advertising has begun.
    //   - onStartFailure: logs the error code for diagnostics.
    // -------------------------------------------------------------------------------------
    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
            Log.d(TAG, "Advertising started: $settingsInEffect")
        }
        override fun onStartFailure(errorCode: Int) {
            Log.e(TAG, "Advertising failed, errorCode=$errorCode")
        }
    }

    // -------------------------------------------------------------------------------------
    // gattServerCallback: Receives GATT server events from BLE centrals.
    //   - onConnectionStateChange: tracks connect/disconnect to manage notifications.
    //   - onDescriptorWriteRequest: handles writes to the CCCD (0x2902) to enable notifications.
    //   - onCharacteristicReadRequest: serves the current JSON payload on read.
    // -------------------------------------------------------------------------------------
    private val gattServerCallback = object : BluetoothGattServerCallback() {
        @SuppressLint("MissingPermission")
        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            Log.d(TAG, "Connection state change: $device status=$status newState=$newState")
            when (newState) {
                BluetoothProfile.STATE_CONNECTED    -> connectedDevices += device.also { Log.d(TAG, "Device connected: $it") }
                BluetoothProfile.STATE_DISCONNECTED -> connectedDevices -= device.also { Log.d(TAG, "Device disconnected: $it") }
            }
        }

        @SuppressLint("MissingPermission")
        override fun onDescriptorWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray
        ) {
            Log.d(TAG, "Descriptor write request: uuid=${descriptor.uuid}, value=${value.contentToString()}")
            // Respond with success to allow notifications to be enabled
            gattServer.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, value)
        }

        @SuppressLint("MissingPermission")
        override fun onCharacteristicReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic
        ) {
            Log.d(TAG, "Read request from $device, reqId=$requestId")
            gattServer.sendResponse(
                device,
                requestId,
                BluetoothGatt.GATT_SUCCESS,
                /* offset= */ 0,
                lastValue
            )
            Log.d(TAG, "Sent response: ${String(lastValue, Charsets.UTF_8)}")
        }
    }
}
