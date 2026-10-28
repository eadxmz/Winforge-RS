use anyhow::Result;
use encoding_rs::{UTF_16LE, UTF_8, WINDOWS_1252};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DriverOrigin {
    HostInstalled,
    UserCustomOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverEntry {
    pub inf_name: String,
    pub inf_path: PathBuf,
    pub provider: String,
    pub class_name: String,
    pub date: String,
    pub version: String,
    pub hardware_ids: Vec<String>,
    pub origin: DriverOrigin,
    pub selected: bool,
}

impl DriverEntry {
    pub fn parse_from_inf(path: &Path, origin: DriverOrigin) -> Result<Self> {
        let bytes = fs::read(path)?;
        let content = decode_inf_bytes(&bytes);

        let mut provider = String::from("Unknown");
        let mut class_name = String::from("System");
        let mut date = String::new();
        let mut version = String::new();
        let mut hardware_ids = Vec::new();

        // Match Hardware IDs like PCI\VEN_1002&DEV_6600 or USB\VID_...
        let hwid_regex = Regex::new(r"(?i)(PCI|USB|ACPI|HDAUDIO)\\[A-Z0-9&_]+")?;

        for line in content.lines() {
            let clean = line.split(';').next().unwrap_or("").trim(); // Strip comments
            if clean.is_empty() {
                continue;
            }

            if let Some((key, val)) = clean.split_once('=') {
                let k = key.trim().to_lowercase();
                let v = val.trim().trim_matches('"');

                match k.as_str() {
                    "class" => class_name = v.to_string(),
                    "provider" => provider = v.trim_start_matches('%').trim_end_matches('%').to_string(),
                    "driverver" => {
                        if let Some((d, ver)) = v.split_once(',') {
                            date = d.trim().to_string();
                            version = ver.trim().to_string();
                        }
                    }
                    _ => {}
                }
            }

            for mat in hwid_regex.find_iter(clean) {
                let id = mat.as_str().to_uppercase();
                if !hardware_ids.contains(&id) {
                    hardware_ids.push(id);
                }
            }
        }

        Ok(Self {
            inf_name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
            inf_path: path.to_path_buf(),
            provider,
            class_name,
            date,
            version,
            hardware_ids,
            origin,
            selected: true,
        })
    }
}

fn decode_inf_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (cow, _, _) = UTF_16LE.decode(&bytes[2..]);
        cow.into_owned()
    } else if let Ok(utf8) = std::str::from_utf8(bytes) {
        utf8.to_string()
    } else {
        let (cow, _, _) = WINDOWS_1252.decode(bytes);
        cow.into_owned()
    }
}

/// Replaces any auto-detected driver that matches Hardware IDs (or Class + Provider) with the user's custom driver
pub fn override_driver(catalog: &mut Vec<DriverEntry>, custom_inf_path: &Path) -> Result<()> {
    let custom_drv = DriverEntry::parse_from_inf(custom_inf_path, DriverOrigin::UserCustomOverride)?;

    for drv in catalog.iter_mut() {
        let hwid_match = drv
            .hardware_ids
            .iter()
            .any(|id| custom_drv.hardware_ids.contains(id));

        let same_display_vendor = drv.class_name.eq_ignore_ascii_case("Display")
            && custom_drv.class_name.eq_ignore_ascii_case("Display")
            && drv.provider.to_lowercase().contains("amd")
            && custom_drv.provider.to_lowercase().contains("amd");

        if hwid_match || same_display_vendor {
            drv.selected = false; // Uncheck the host's version automatically
        }
    }

    catalog.push(custom_drv);
    Ok(())
}
