//! Desktop-only real OS system utilities: clipboard, device info, permissions.
//!
//! Uses `arboard` for clipboard access, `sysinfo` for hardware/OS info,
//! and `enigo` for accessibility permission detection on macOS.

use crate::{PermissionStatus, ToolResult};
use sysinfo::System;

// ---------------------------------------------------------------------------
// Clipboard operations
// ---------------------------------------------------------------------------

/// Read the current text content from the system clipboard.
///
/// Returns the clipboard text, or an appropriate error if the clipboard
/// is empty or unavailable.
pub fn do_clipboard_get() -> ToolResult {
    log::info!("do_clipboard_get: reading system clipboard");

    let result = (|| -> Result<String, String> {
        let mut ctx = arboard::Clipboard::new()
            .map_err(|e| format!("Failed to access clipboard: {}", e))?;
        match ctx.get_text() {
            Ok(text) => Ok(text),
            Err(arboard::Error::ContentNotAvailable) => {
                Ok(String::new()) // Empty clipboard is not an error
            }
            Err(e) => Err(format!("Failed to read clipboard: {}", e)),
        }
    })();

    match result {
        Ok(text) => {
            log::info!(
                "do_clipboard_get: success — {} chars",
                text.len()
            );
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": "Clipboard content retrieved",
                    "text": text,
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_clipboard_get: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Set the system clipboard to the given text.
pub fn do_clipboard_set(text: &str) -> ToolResult {
    log::info!("do_clipboard_set: setting clipboard ({} chars)", text.len());

    if text.trim().is_empty() {
        return ToolResult {
            success: false,
            data: None,
            error: Some("text parameter required for 'set' action".into()),
        };
    }

    let result = (|| -> Result<(), String> {
        let mut ctx = arboard::Clipboard::new()
            .map_err(|e| format!("Failed to access clipboard: {}", e))?;
        ctx.set_text(text)
            .map_err(|e| format!("Failed to set clipboard: {}", e))?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info!("do_clipboard_set: success — {} chars set", text.len());
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": format!("Clipboard set to: '{}'", text),
                    "text": text,
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_clipboard_set: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Device info
// ---------------------------------------------------------------------------

/// Query real OS/hardware information by category.
///
/// - "device": OS name+version, CPU brand, total RAM
/// - "storage": per-disk mount point, total, available space
/// - "time": current system time (UTC)
/// - "battery", "wifi", "bluetooth", "screen": not available on desktop
/// - unknown category: error listing supported categories
pub fn do_get_device_info(category: &str) -> ToolResult {
    log::info!("do_get_device_info: category='{}'", category);

    let result = match category.to_lowercase().as_str() {
        "device" => {
            let mut sys = sysinfo::System::new_all();
            sys.refresh_all();

            let os_name = System::long_os_version()
                .unwrap_or_else(|| "Unknown OS".to_string());
            let cpu_brand = if let Some(cpu) = sys.cpus().first() {
                cpu.brand().to_string()
            } else {
                "Unknown CPU".to_string()
            };
            let total_ram_gb = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

            let info = format!(
                "{}, CPU: {}, RAM: {:.1} GB",
                os_name,
                cpu_brand,
                total_ram_gb
            );
            Ok(serde_json::json!({
                "info": info,
                "os": os_name,
                "cpu": cpu_brand,
                "ram_gb": (total_ram_gb * 10.0).round() / 10.0,
            }))
        }
        "storage" => {
            let disks = sysinfo::Disks::new_with_refreshed_list();
            let disk_list: Vec<serde_json::Value> = disks
                .iter()
                .map(|d| {
                    let total_gb = d.total_space() as f64 / (1024.0 * 1024.0 * 1024.0);
                    let available_gb = d.available_space() as f64 / (1024.0 * 1024.0 * 1024.0);
                    serde_json::json!({
                        "mount": d.mount_point().to_string_lossy(),
                        "name": d.name().to_string_lossy(),
                        "total_gb": (total_gb * 10.0).round() / 10.0,
                        "available_gb": (available_gb * 10.0).round() / 10.0,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "info": format!("{} disk(s) found", disk_list.len()),
                "disks": disk_list,
            }))
        }
        "time" => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let days_since_epoch = secs / 86400;
            let time_of_day = secs % 86400;
            let hours = (time_of_day / 3600) as u32;
            let minutes = ((time_of_day % 3600) / 60) as u32;
            let seconds = (time_of_day % 60) as u32;

            // Howard Hinnant's civil_from_days algorithm
            let z = days_since_epoch as i64 + 719468;
            let era = if z >= 0 { z } else { z - 146096 } / 146097;
            let doe = z - era * 146097;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
            let y = yoe + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
            let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
            let y = (y + if m <= 2 { 1 } else { 0 }) as u32;

            let time_str = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                y, m, d, hours, minutes, seconds
            );
            Ok(serde_json::json!({
                "info": format!("Time: {}", time_str),
                "time": time_str,
            }))
        }
        "battery" | "wifi" | "bluetooth" | "screen" => {
            Ok(serde_json::json!({
                "info": "Not available on desktop",
                "category": category,
            }))
        }
        _ => {
            return ToolResult {
                success: false,
                data: None,
                error: Some(format!(
                    "Unknown category '{}'. Supported: device, storage, time, battery, wifi, bluetooth, screen",
                    category
                )),
            };
        }
    };

    match result {
        Ok(data) => {
            log::info!("do_get_device_info: success — category='{}'", category);
            ToolResult {
                success: true,
                data: Some(data),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_get_device_info: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Permission checks
// ---------------------------------------------------------------------------

/// Check desktop permission status.
///
/// On desktop, most permissions are trivially available. The notable
/// exception is macOS accessibility: creating an `Enigo` instance will
/// fail if the app hasn't been granted Accessibility permission in
/// System Settings > Privacy & Security.
///
/// Returns a `PermissionStatus` struct matching the Android contract.
pub fn do_check_permissions() -> ToolResult {
    log::info!("do_check_permissions: checking desktop permissions");

    // On macOS, try creating an Enigo to detect accessibility permission.
    // On Windows/Linux, this always succeeds.
    let accessibility_enabled = check_enigo_available();

    let status = PermissionStatus {
        accessibility_enabled,
        accessibility_running: accessibility_enabled,
        notification_enabled: true,   // Desktop always has notification access
        foreground_service: true,     // Desktop always can run foreground tasks
    };

    log::info!(
        "do_check_permissions: accessibility_enabled={}, notification_enabled=true, foreground_service=true",
        accessibility_enabled
    );

    ToolResult {
        success: true,
        data: Some(serde_json::to_value(&status).unwrap_or_else(|_| serde_json::json!({}))),
        error: None,
    }
}

/// Try creating an Enigo instance to check if accessibility/input
/// permissions are available. Returns true if Enigo can be created,
/// false otherwise (e.g., macOS without Accessibility permission).
fn check_enigo_available() -> bool {
    use enigo::{Enigo, Settings};
    match Enigo::new(&Settings::default()) {
        Ok(_) => true,
        Err(e) => {
            log::warn!(
                "check_enigo_available: Enigo creation failed — {}. \
                 On macOS, grant Accessibility permission in System Settings > Privacy & Security.",
                e
            );
            false
        }
    }
}
