//! FIDO2/WebAuthn + U2F security-key support (SSH certificates), mirroring
//! `@termius/libfido2`.
//!
//! Termius uses a FIDO2/U2F security key (and platform authenticators such as
//! Windows Hello) to unlock SSH keys and authenticate (`fido2BasedKey`,
//! resident passkeys, the FIDO2 keygen wizard). In the Electron app this runs
//! through the native `@termius/libfido2` addon — a Node binding over C
//! `libfido2` 1.10 plus OpenSSH's `ssh-sk` enroll/sign helpers (symbols
//! `__ssh_sk_enroll`, `__ssh_sk_sign`, `SignData`, `GetDevicesInfo` recovered
//! from the shipped `libfido2-nodejs.node`).
//!
//! This crate currently provides the **stable API surface** plus honest
//! stubs: the data types and error classification are real and serde-compatible
//! with Termius' JS wire format (camelCase + standard base64), while the
//! hardware-facing operations — [`device::FidoDevice::list_devices`],
//! [`device::FidoDevice::open`], [`auth::FidoAuthenticator::assert`] and
//! [`auth::FidoAuthenticator::make_credential`] — return
//! [`error::FidoError::Unsupported`] / [`error::FidoError::NotImplemented`]
//! until a CTAP2/HID backend is ported (see the `PORT-TODO`s in `device.rs`
//! and `auth.rs`). No crypto is faked: nothing here ever returns a fabricated
//! credential or signature.
//!
//! Module tree:
//! * [`error`] — `FidoError` variants + backend-message classification,
//!   `Result` alias.
//! * [`device`] — `FidoDevice` discovery/handle and `DeviceInfo` metadata.
//! * [`auth`] — `FidoAuthenticator` operations and WebAuthn data types
//!   (`RelyingParty`, `Assertion`, `PublicKeyCredential`, `UserHandle`).
//! * `serde_util` (private) — JS-compatible base64 codec for byte fields.

pub mod auth;
pub mod device;
pub mod error;

mod serde_util;

pub use auth::{
    Assertion, CredentialAlgorithm, FidoAuthenticator, PublicKeyCredential, RelyingParty,
    UserHandle,
};
pub use device::{DeviceInfo, DeviceTransport, FidoDevice};
pub use error::{FidoError, FidoErrorKind, Result};
