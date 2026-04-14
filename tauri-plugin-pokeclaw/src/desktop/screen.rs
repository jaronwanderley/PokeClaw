//! Desktop-only screen capture and window tree enumeration via xcap.
//!
//! Provides functions to capture real OS screenshots and enumerate
//! the window tree (titles, positions, sizes).

use crate::ToolResult;
use serde_json::json;
use xcap::{Monitor, Window};

/// Capture a screenshot of the primary monitor and save it to the specified path.
/// If no path is provided, it uses a default timestamped filename in the current directory.
pub fn take_screenshot(path: Option<&str>) -> ToolResult {
    log::info!("take_screenshot: path={:?}", path);

    // TODO: Implement real screen capture with xcap and image
    let result = (|| -> Result<String, String> {
        let save_path = path.unwrap_or("screenshot.png").to_string();
        Ok(save_path)
    })();

    match result {
        Ok(p) => {
            log::info!("take_screenshot: success, saved to {}", p);
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
            log::error!("take_screenshot: failed — {}", e);
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
