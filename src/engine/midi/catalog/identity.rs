//! Exact Linux USB identity and ALSA port pairing, discovered off callbacks.
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Device {
    pub vendor: u16,
    pub product: u16,
    pub release: u16,
    pub serial: Option<String>,
    pub topology: String,
    pub port: u8,
    pub connection: String,
}
fn attribute(path: PathBuf) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(257)
        .read_to_end(&mut bytes)
        .ok()?;
    let value = String::from_utf8(bytes).ok()?.trim().to_owned();
    (!value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
        .then_some(value)
}
fn serial(value: Option<String>, product: Option<String>) -> Option<String> {
    value.filter(|v| {
        !v.eq_ignore_ascii_case("no serial number")
            && product.as_ref().is_none_or(|p| !v.eq_ignore_ascii_case(p))
    })
}
impl Device {
    /// Inspect one ALSA hardware port through its owning card.
    /// Takes the exact client:port ID; returns physical USB attributes and instance topology or no identity for virtual/unknown ports.
    pub fn discover(id: &str) -> Option<Self> {
        let (client, port) = id.split_once(':')?;
        let client = client.parse::<i32>().ok()?;
        let port = port.parse::<u8>().ok()?;
        let seq = alsa::seq::Seq::open(None, None, true).ok()?;
        seq.set_client_name(c"omatainer-identity").ok()?;
        let card = seq.get_any_client_info(client).ok()?.get_card().ok()?;
        if card < 0 || card > 4095 {
            return None;
        }
        let mut path = std::fs::canonicalize(format!("/sys/class/sound/card{card}/device")).ok()?;
        for _ in 0..16 {
            if let (Some(vendor), Some(product), Some(release)) = (
                attribute(path.join("idVendor")),
                attribute(path.join("idProduct")),
                attribute(path.join("bcdDevice")),
            ) {
                return Some(Self {
                    vendor: u16::from_str_radix(&vendor, 16).ok()?,
                    product: u16::from_str_radix(&product, 16).ok()?,
                    release: u16::from_str_radix(&release, 16).ok()?,
                    serial: serial(
                        attribute(path.join("serial")),
                        attribute(path.join("product")),
                    ),
                    topology: path.file_name()?.to_str()?.into(),
                    port,
                    connection: format!(
                        "{}:{}",
                        attribute(path.join("busnum"))?,
                        attribute(path.join("devnum"))?
                    ),
                });
            }
            if !path.pop() {
                break;
            }
        }
        None
    }
    /// Preserve an exact physical instance and port role across ALSA renumbering.
    /// Takes these observed attributes; returns a serial-qualified or topology-qualified private instance key without inferring one controller from another's name.
    pub fn key(&self) -> String {
        format!(
            "usb:{:04x}:{:04x}:{}:{}",
            self.vendor,
            self.product,
            self.serial.as_ref().map_or_else(
                || format!("topology:{}", self.topology),
                |s| format!("serial:{s}")
            ),
            self.port
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_labels_and_missing_serial_placeholders_never_anchor_an_instance() {
        for value in ["NO SERIAL NUMBER", "no serial number", "Pioneer DDJ-SP1"] {
            assert!(serial(Some(value.into()), Some("Pioneer DDJ-SP1".into())).is_none());
        }
        assert_eq!(
            serial(Some("unit-0123".into()), Some("Pioneer DDJ-SP1".into())),
            Some("unit-0123".into())
        );
    }
}
