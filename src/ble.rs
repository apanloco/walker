use btleplug::api::{Central, CharPropFlags, Characteristic, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use std::time::Duration;
use tracing::{info, warn};

use crate::device::{ProfileRegistry, TreadmillProfile};

/// Check Bluetooth permission on macOS. Fails early with a clear message
/// instead of letting CoreBluetooth segfault when permission is denied.
#[cfg(target_os = "macos")]
pub fn check_bluetooth_permission() -> anyhow::Result<()> {
    // Call [CBManager authorization] via Objective-C runtime FFI.
    // Returns: 0=notDetermined, 1=restricted, 2=denied, 3=allowed
    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
        fn objc_msgSend(receiver: *const std::ffi::c_void, sel: *const std::ffi::c_void) -> isize;
    }

    let auth = unsafe {
        let class = objc_getClass(c"CBManager".as_ptr());
        if class.is_null() {
            return Ok(()); // CoreBluetooth not available — skip check.
        }
        let sel = sel_registerName(c"authorization".as_ptr());
        objc_msgSend(class, sel)
    };

    match auth {
        2 => anyhow::bail!(
            "Bluetooth permission denied. Grant access in System Settings > Privacy & Security > Bluetooth."
        ),
        1 => anyhow::bail!("Bluetooth access is restricted on this device."),
        _ => Ok(()), // 0=notDetermined (will prompt), 3=allowed
    }
}

#[cfg(not(target_os = "macos"))]
pub fn check_bluetooth_permission() -> anyhow::Result<()> {
    Ok(())
}

pub async fn get_adapter() -> anyhow::Result<Adapter> {
    check_bluetooth_permission()?;
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;

    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Bluetooth adapters found"))?;

    info!(adapter = ?adapter.adapter_info().await?, "Using adapter");
    Ok(adapter)
}

pub async fn scan(
    adapter: &Adapter,
    timeout: u64,
) -> anyhow::Result<Vec<btleplug::platform::Peripheral>> {
    adapter.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(Duration::from_secs(timeout)).await;
    adapter.stop_scan().await?;
    Ok(adapter.peripherals().await?)
}

/// Check already-known peripherals for a matching treadmill (no scan needed).
async fn check_known<'a>(
    adapter: &Adapter,
    registry: &'a ProfileRegistry,
) -> anyhow::Result<Option<(btleplug::platform::Peripheral, &'a dyn TreadmillProfile)>> {
    let peripherals = adapter.peripherals().await?;
    for peripheral in peripherals {
        if let Some(props) = peripheral.properties().await? {
            let name = props.local_name.as_deref();
            if let Some(profile) = registry.match_device(name, &props.services) {
                return Ok(Some((peripheral, profile)));
            }
        }
    }
    Ok(None)
}

/// Scan for a treadmill that matches any registered profile.
/// Does a quick 1-second sweep first, then falls back to full scan.
pub async fn find_treadmill<'a>(
    adapter: &Adapter,
    timeout: u64,
    registry: &'a ProfileRegistry,
) -> anyhow::Result<Option<(btleplug::platform::Peripheral, &'a dyn TreadmillProfile)>> {
    // Quick check: maybe the adapter already knows about the device.
    info!("Quick scan (1 second)...");
    adapter.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    adapter.stop_scan().await?;

    if let Some(found) = check_known(adapter, registry).await? {
        info!("Found device on quick scan!");
        return Ok(Some(found));
    }

    // Full scan.
    info!("Full scan ({timeout} seconds)...");
    let peripherals = scan(adapter, timeout).await?;

    for peripheral in peripherals {
        if let Some(props) = peripheral.properties().await? {
            let name = props.local_name.as_deref();
            if let Some(profile) = registry.match_device(name, &props.services) {
                return Ok(Some((peripheral, profile)));
            }
        }
    }

    Ok(None)
}

/// Subscribe to the specific notification characteristics a profile needs.
pub async fn subscribe_notify(
    device: &btleplug::platform::Peripheral,
    notify_uuids: &[uuid::Uuid],
) -> anyhow::Result<Vec<Characteristic>> {
    let services = device.services();
    let mut subscribed = Vec::new();

    for service in &services {
        for ch in &service.characteristics {
            if notify_uuids.contains(&ch.uuid) && ch.properties.contains(CharPropFlags::NOTIFY) {
                match device.subscribe(ch).await {
                    Ok(()) => {
                        subscribed.push(ch.clone());
                    }
                    Err(e) => {
                        warn!(uuid = %ch.uuid, error = %e, "Failed to subscribe");
                    }
                }
            }
        }
    }

    info!(
        "Subscribed to {} notification characteristic(s)",
        subscribed.len()
    );
    Ok(subscribed)
}
