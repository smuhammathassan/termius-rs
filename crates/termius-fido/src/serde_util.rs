//! Private serde helpers: JS-compatible standard base64 for byte fields.
//!
//! Termius' JS layer always transports binary FIDO material (key handles,
//! challenges, authenticator data, signatures, public keys) as base64 strings
//! — see `toString("base64")` / `Buffer.from(...)` in the recovered sources.
//! A tiny self-contained codec keeps this crate free of extra dependencies
//! while producing wire-compatible JSON. It is encoding, not cryptography.

use serde::{Deserialize, Serializer};

const ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `#[serde(with = "crate::serde_util")]` serializer: bytes -> base64 text.
pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&encode(bytes))
}

/// `#[serde(with = "crate::serde_util")]` deserializer: base64 text -> `Vec<u8>`.
pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    decode(&text).map_err(serde::de::Error::custom)
}

/// Standard base64 (RFC 4648 §4) with padding — the alphabet `Buffer#toString("base64")` uses.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(if chunk.len() > 1 { chunk[1] } else { 0 });
        let b2 = u32::from(if chunk.len() > 2 { chunk[2] } else { 0 });
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(b2 & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Decode standard base64. Errors are plain strings so callers can map them
/// onto `serde::de::Error` (deserialization) or `FidoError::Protocol`.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "invalid base64 length {} (must be a multiple of 4)",
            bytes.len()
        ));
    }
    let quad_count = bytes.len() / 4;
    let mut out = Vec::with_capacity(quad_count * 3);
    for (index, quad) in bytes.chunks(4).enumerate() {
        let is_last = index + 1 == quad_count;
        let mut values = [0u8; 4];
        let mut padding = 0usize;
        for (position, &byte) in quad.iter().enumerate() {
            if byte == b'=' {
                if !is_last || position < 2 {
                    return Err("misplaced '=' padding in base64 input".to_string());
                }
                padding += 1;
            } else {
                if padding > 0 {
                    return Err("base64 data after '=' padding".to_string());
                }
                values[position] = decode_char(byte)?;
            }
        }
        out.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            out.push(((values[1] & 0x0F) << 4) | (values[2] >> 2));
        }
        if padding < 1 {
            out.push(((values[2] & 0x03) << 6) | values[3]);
        }
    }
    Ok(out)
}

fn decode_char(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(format!("invalid base64 character {:?}", char::from(byte))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors_encode() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(input.as_bytes()), expected, "encode({input:?})");
        }
    }

    #[test]
    fn rfc4648_vectors_decode() {
        for (expected, input) in [
            ("", ""),
            ("Zg==", "f"),
            ("Zm8=", "fo"),
            ("Zm9v", "foo"),
            ("Zm9vYg==", "foob"),
            ("Zm9vYmE=", "fooba"),
            ("Zm9vYmFy", "foobar"),
        ] {
            assert_eq!(decode(expected).expect("valid vector"), input.as_bytes().to_vec());
        }
    }

    #[test]
    fn round_trip_every_byte_value() {
        let data: Vec<u8> = (0u8..=255).collect();
        let text = encode(&data);
        assert_eq!(decode(&text).expect("round-trip"), data);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in ["Zg=", "Z===", "Zg==Zg==", "Zg!=", "abc", "Zm9v "] {
            assert!(decode(bad).is_err(), "expected error for {bad:?}");
        }
    }
}
