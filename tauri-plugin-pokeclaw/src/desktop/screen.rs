//! Desktop-only screen capture and window tree enumeration via xcap.
//!
//! Provides functions to capture real OS screenshots and enumerate
//! the window tree (titles, positions, sizes).

use crate::ToolResult;
use serde_json::json;
use xcap::Monitor;

/// Capture a screenshot of the primary monitor and save it to the specified path.
/// If no path is provided, it uses a default timestamped filename in the current directory.
pub fn do_take_screenshot(path: Option<&str>) -> ToolResult {
    log::info!("do_take_screenshot: path={:?}", path);

    let result = (|| -> Result<String, String> {
        let monitors = Monitor::all().map_err(|e| format!("Failed to get monitors: {}", e))?;
        let primary_monitor = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .ok_or_else(|| "No primary monitor found".to_string())?;

        let image = primary_monitor
            .capture_image()
            .map_err(|e| format!("Failed to capture image: {}", e))?;

        let save_path = match path {
            Some(p) => p.to_string(),
            None => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("screenshot_{}.png", now)
            }
        };
        image
            .save(&save_path)
            .map_err(|e| format!("Failed to save image to {}: {}", save_path, e))?;

        Ok(save_path)
    })();

    match result {
        Ok(p) => {
            log::info!("do_take_screenshot: success, saved to {}", p);
            ToolResult {
                success: true,
                data: Some(json!({
                    "message": format!("Screenshot captured and saved to {}", p),
                    "path": p
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_take_screenshot: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Enumerate all visible windows and return their metadata (title, position, size).
pub fn get_screen_info() -> ToolResult {
    log::info!("get_screen_info");

    // TODO: Implement real window enumeration with xcap
    let result = (|| -> Result<serde_json::Value, String> {
        Ok(json!([]))
    })();

    match result {
        Ok(windows) => {
            log::info!("get_screen_info: success, found {} windows", windows.as_array().map_or(0, |a| a.len()));
            ToolResult {
                success: true,
                data: Some(json!({
                    "windows": windows
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("get_screen_info: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}
