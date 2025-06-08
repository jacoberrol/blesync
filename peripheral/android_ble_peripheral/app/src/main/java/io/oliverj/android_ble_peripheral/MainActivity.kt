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

    private lateinit var advertiser: BluetoothLeAdvertiser
    private lateinit var gattServer: BluetoothGattServer
    private lateinit var char: BluetoothGattCharacteristic
    private val handler = Handler(Looper.getMainLooper())
    private var counter = 0

    private val connectedDevices = mutableSetOf<BluetoothDevice>()
    private var lastValue: ByteArray = ByteArray(0)

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

    private fun hasAllPermissions(): Boolean =
        REQUIRED_PERMISSIONS.all { perm ->
            ContextCompat.checkSelfPermission(this, perm) == PackageManager.PERMISSION_GRANTED
        }

    @SuppressLint("MissingPermission")
    override fun onDestroy() {
        super.onDestroy()
        Log.d(TAG, "onDestroy(): stopping advertising and closing GATT")
        advertiser.stopAdvertising(advertiseCallback)
        gattServer.close()
        handler.removeCallbacksAndMessages(null)
    }

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

    @SuppressLint("MissingPermission")
    private fun startBlePeripheral() {
        val btManager = getSystemService(BLUETOOTH_SERVICE) as BluetoothManager
        val adapter = btManager.adapter

        Log.d(TAG, "Adapter name=${adapter.name}, address=${adapter.address}")
        Log.d(TAG, "Supports peripheral mode? ${adapter.isMultipleAdvertisementSupported}")

        advertiser = adapter.bluetoothLeAdvertiser
        gattServer = btManager.openGattServer(this, gattServerCallback)

        // 1) Create the characteristic with READ + NOTIFY
        char = BluetoothGattCharacteristic(
            CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ
        )

        // 2) Add the CCCD descriptor so centrals can enable notifications
        val cccdUuid = UUID.fromString("00002902-0000-1000-8000-00805F9B34FB")
        val cccd = BluetoothGattDescriptor(
            cccdUuid,
            BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE
        )
        char.addDescriptor(cccd)
        Log.d(TAG, "Added CCCD descriptor: $cccdUuid")

        // 3) Create and register the service
        val service = BluetoothGattService(
            SERVICE_UUID,
            BluetoothGattService.SERVICE_TYPE_PRIMARY
        )
        service.addCharacteristic(char)
        gattServer.addService(service)
        Log.d(TAG, "Service added: $SERVICE_UUID, Characteristic: $CHAR_UUID")

        // 4) Prepare advertising data (only the service UUID to stay under 31 bytes)
        val advertiseData = AdvertiseData.Builder()
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()

        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setConnectable(true)
            .build()

        Log.d(TAG, "Starting advertising with data: $advertiseData")
        advertiser.startAdvertising(settings, advertiseData, advertiseCallback)

        // 5) Kick off periodic notifications
        handler.postDelayed(::updateCharacteristic, 5000)
    }


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

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
            Log.d(TAG, "Advertising started: $settingsInEffect")
        }
        override fun onStartFailure(errorCode: Int) {
            Log.e(TAG, "Advertising failed, errorCode=$errorCode")
        }
    }

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
            // Echo it back to the client
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
