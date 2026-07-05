//! Stable hardware/software fingerprint → SHA256 hex (device_id). Not stored on disk.

use std::collections::BTreeMap;

#[cfg(target_os = "windows")]
use std::collections::HashMap;

#[cfg(target_os = "linux")]
use std::fs;

use sha2::{Digest, Sha256};

/// Compute device_id from a canonical stable machine snapshot (64-char hex SHA256).
pub fn compute_device_id() -> anyhow::Result<String> {
    let mut pairs = BTreeMap::new();
    pairs.insert("target_os".to_string(), std::env::consts::OS.to_string());
    pairs.insert(
        "target_arch".to_string(),
        std::env::consts::ARCH.to_string(),
    );

    collect_platform_fields(&mut pairs)?;

    if pairs.len() <= 2 {
        anyhow::bail!("Insufficient fingerprint fields for device_id");
    }

    let canonical = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("|");

    Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
}

fn collect_platform_fields(pairs: &mut BTreeMap<String, String>) -> anyhow::Result<()> {
    if let Ok(uid) = machine_uid::get() {
        let uid = uid.trim();
        if !uid.is_empty() {
            pairs.insert("machine_uid".to_string(), uid.to_string());
        }
    }

    #[cfg(target_os = "windows")]
    collect_windows(pairs)?;

    #[cfg(target_os = "linux")]
    collect_linux(pairs)?;

    #[cfg(target_os = "macos")]
    collect_macos(pairs)?;

    Ok(())
}

fn is_generic_serial(value: &str) -> bool {
    let v = value.trim();
    v.is_empty()
        || v.eq_ignore_ascii_case("System Serial Number")
        || v.eq_ignore_ascii_case("(empty)")
        || v == "To be filled by O.E.M."
        || v == "Default string"
}

#[cfg(target_os = "linux")]
fn read_trim(path: &str) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    if s.is_empty() || is_generic_serial(s) {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(target_os = "linux")]
fn collect_linux(pairs: &mut BTreeMap<String, String>) -> anyhow::Result<()> {
    if let Some(uuid) = read_trim("/sys/class/dmi/id/product_uuid") {
        pairs.insert("product_uuid".to_string(), uuid);
    }
    if let Some(serial) = read_trim("/sys/class/dmi/id/board_serial") {
        pairs.insert("board_serial".to_string(), serial);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn collect_windows(pairs: &mut BTreeMap<String, String>) -> anyhow::Result<()> {
    use wmi::{COMLibrary, WMIConnection};

    let com = COMLibrary::new()?;
    let wmi = WMIConnection::new(com.into())?;

    if let Ok(rows) = wmi_query_one(&wmi, "SELECT ProcessorId FROM Win32_Processor") {
        if let Some(v) = rows.get("ProcessorId") {
            if let Some(s) = variant_str(v) {
                if !s.is_empty() {
                    pairs.insert("processor_id".to_string(), s);
                }
            }
        }
    }

    if let Ok(rows) = wmi_query_one(
        &wmi,
        "SELECT UUID, Vendor FROM Win32_ComputerSystemProduct",
    ) {
        if let Some(v) = rows.get("UUID") {
            if let Some(s) = variant_str(v) {
                if !s.is_empty() {
                    pairs.insert("system_uuid".to_string(), s);
                }
            }
        }
        if let Some(v) = rows.get("Vendor") {
            if let Some(s) = variant_str(v) {
                if !s.is_empty() {
                    pairs.insert("system_vendor".to_string(), s);
                }
            }
        }
    }

    if let Ok(rows) = wmi_query_one(&wmi, "SELECT SerialNumber FROM Win32_BaseBoard") {
        if let Some(v) = rows.get("SerialNumber") {
            if let Some(s) = variant_str(v) {
                if !is_generic_serial(&s) {
                    pairs.insert("baseboard_serial".to_string(), s);
                }
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn wmi_query_one(
    wmi: &wmi::WMIConnection,
    query: &str,
) -> anyhow::Result<HashMap<String, wmi::Variant>> {
    let results: Vec<HashMap<String, wmi::Variant>> = wmi.raw_query(query)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("WMI empty result"))
}

#[cfg(target_os = "windows")]
fn variant_str(v: &wmi::Variant) -> Option<String> {
    use wmi::Variant;
    match v {
        Variant::String(s) => Some(s.trim().to_string()),
        Variant::I4(n) => Some(n.to_string()),
        Variant::UI4(n) => Some(n.to_string()),
        Variant::I8(n) => Some(n.to_string()),
        Variant::UI8(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn collect_macos(pairs: &mut BTreeMap<String, String>) -> anyhow::Result<()> {
    // IOPlatformUUID via ioreg — optional for v1 without hw-probe on macOS
    if let Ok(out) = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains("IOPlatformUUID") {
                if let Some(uuid) = line.split('"').nth(3) {
                    let uuid = uuid.trim();
                    if !uuid.is_empty() {
                        pairs.insert("platform_uuid".to_string(), uuid.to_string());
                    }
                }
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable_hex() {
        let id = compute_device_id().expect("device_id");
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, compute_device_id().expect("device_id"));
    }
}
