//! FIDO2/U2F security-key discovery and handle.
//!
//! Mirrors the surface Termius calls through `@termius/libfido2`
//! (`getDevicesInfo`, `getAmountDevices`, `getDetailedDeviceInfo`), modelled on
//! the field names recovered from the shipped `mac-arm64/libfido2-nodejs.node`
//! addon (`path`, `product`, `manufacturer`, `isPinRequired`, `pinAttempts`,
//! `isUserPresenceRequired`).
//!
//! PORT-TODO: no CTAP2/HID backend is linked yet. `list_devices` and `open`
//! return `Unsupported` / `NotImplemented` instead of panicking, so the SSH
//! and UI crates can compile against this API today. Back the transport with
//! one of:
//!   * `webauthn-authenticator-rs` (pure Rust CTAP2/U2F client, hidapi transport)
//!   * `ctap-hid-fido2` (pure Rust CTAP2 over HID)
//!   * FFI bindings to C `libfido2` — exactly what @termius/libfido2 ships
//!     (libfido2 1.10.0 + OpenSSH `ssh-sk` helpers compiled into the addon)
//! CI already installs `libudev-dev`, so a hidapi-based backend will link.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{FidoError, Result};

/// Transport a security key is reachable over (libfido2 device transports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceTransport {
    /// FIDO2/U2F over USB HID (the common case).
    Usb,
    /// FIDO2 over NFC.
    Nfc,
    /// FIDO2 over BLE.
    Ble,
    /// Backend could not determine the transport (or none linked).
    #[default]
    Unknown,
}

/// Static metadata for one attached security key — one entry of the addon's
/// `getDevicesInfo()`, camelCase on the wire to match Termius' JS field names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// HID path / selector used to reopen the device (JS `path`).
    pub path: String,
    /// Product string reported by the device (JS `product`).
    pub product: String,
    /// Manufacturer string reported by the device (JS `manufacturer`).
    pub manufacturer: String,
    /// Whether the next operation will ask for a PIN (JS `isPinRequired`).
    #[serde(default)]
    pub pin_required: bool,
    /// Remaining PIN retries when the device reports them (JS `pinAttempts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_attempts: Option<u32>,
    /// Whether a touch (user presence) is required (JS `isUserPresenceRequired`).
    #[serde(default)]
    pub user_presence_required: bool,
    /// Transport the device is reachable over.
    #[serde(default)]
    pub transport: DeviceTransport,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}) [{}]", self.product, self.manufacturer, self.path)
    }
}

/// Handle to one FIDO2/U2F security key.
///
/// Cheap to clone; carries the device metadata returned by discovery. Opening
/// the underlying transport is deferred (see the `PORT-TODO`s below) — the
/// type exists so callers can compile against the final API shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FidoDevice {
    info: DeviceInfo,
}

impl FidoDevice {
    /// Enumerate the security keys attached to this machine.
    ///
    /// PORT-TODO: replace the body with a real CTAP2/HID enumeration
    /// (`fido_dev_info_manifest` via libfido2 FFI, or hidapi device
    /// discovery + CTAP2 `GetInfo`) filling every `DeviceInfo` field.
    pub fn list_devices() -> Result<Vec<DeviceInfo>> {
        // PORT-TODO: replace with real enumeration (libfido2 `fido_dev_info_manifest`
        // FFI, or hidapi + CTAP2 GetInfo). Until then: unsupported, never a panic.
        Err(FidoError::unsupported(
            "device enumeration needs a CTAP2/HID backend \
             (webauthn-authenticator-rs / ctap-hid-fido2 / libfido2 binding) \
             — see PORT-TODO in termius-fido::device",
        ))
    }

    /// Whether at least one security key is attached *and* a backend that can
    /// enumerate it is linked. Always `false` while the backend is deferred;
    /// any enumeration failure is treated as "not available" rather than
    /// propagated, so UI feature-gating stays a cheap boolean.
    pub fn is_available() -> bool {
        match Self::list_devices() {
            Ok(devices) => !devices.is_empty(),
            Err(_) => false,
        }
    }

    /// Open the transport for the device described by `info`.
    ///
    /// PORT-TODO: open the HID device, negotiate CTAP2 (or fall back to
    /// U2F/CBOR transport), and surface user-cancel as
    /// `FidoError::Cancelled` rather than a generic failure.
    pub fn open(info: &DeviceInfo) -> Result<Self> {
        // PORT-TODO: open the HID device + negotiate CTAP2 (U2F fallback), wire
        // cancellation to FidoError::Cancelled. Until then: not implemented.
        Err(FidoError::not_implemented(format!(
            "opening FIDO2 device '{}' (CTAP2 over HID transport)",
            info.path
        )))
    }

    /// Metadata this handle was created from.
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// HID path of this device.
    pub fn path(&self) -> &str {
        &self.info.path
    }

    /// Whether the device expects a PIN prompt before signing.
    pub fn is_pin_required(&self) -> bool {
        self.info.pin_required
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FidoErrorKind;

    fn sample_info() -> DeviceInfo {
        DeviceInfo {
            path: "/dev/hidraw0".to_string(),
            product: "Security Key".to_string(),
            manufacturer: "Test Vendor".to_string(),
            pin_required: true,
            pin_attempts: Some(3),
            user_presence_required: true,
            transport: DeviceTransport::Usb,
        }
    }

    #[test]
    fn list_devices_is_explicitly_unsupported_without_backend() {
        let err = FidoDevice::list_devices().expect_err("no backend is linked");
        assert_eq!(err.kind(), FidoErrorKind::Unsupported);
        assert!(err.to_string().contains("CTAP2/HID backend"));
    }

    #[test]
    fn is_available_is_false_without_backend() {
        assert!(!FidoDevice::is_available());
    }

    #[test]
    fn open_stub_names_the_device_it_refuses() {
        let err = FidoDevice::open(&sample_info()).expect_err("no backend is linked");
        assert_eq!(err.kind(), FidoErrorKind::Unimplemented);
        assert!(err.to_string().contains("/dev/hidraw0"));
    }

    #[test]
    fn device_info_json_round_trip_is_camel_case() {
        let info = sample_info();
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains("\"pinRequired\":true"), "json: {json}");
        assert!(json.contains("\"pinAttempts\":3"), "json: {json}");
        assert!(json.contains("\"userPresenceRequired\":true"), "json: {json}");
        assert!(json.contains("\"transport\":\"usb\""), "json: {json}");

        let back: DeviceInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, info);
    }

    #[test]
    fn device_info_deserializes_without_optional_fields() {
        let minimal = r#"{
            "path": "pc:/dev/hidraw0",
            "product": "Key",
            "manufacturer": "Vendor"
        }"#;
        let info: DeviceInfo = serde_json::from_str(minimal).expect("defaults apply");
        assert!(!info.pin_required);
        assert_eq!(info.pin_attempts, None);
        assert!(!info.user_presence_required);
        assert_eq!(info.transport, DeviceTransport::Unknown);
    }
}
