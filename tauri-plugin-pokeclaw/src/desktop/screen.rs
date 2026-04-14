//! Desktop-only screen capture and window tree enumeration via xcap.
//!
//! Provides functions to capture real OS screenshots and enumerate
//! the window tree (titles, positions, sizes).

use crate::ToolResult;
use serde_json::json;
use xcap::{Monitor, Window};

/// Capture a screenshot of the primary monitor and save it to the specified path.
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
pub fn do_get_screen_info() -> ToolResult {
    log::info!("do_get_screen_info");

    let result = (|| -> Result<String, String> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;
        
        let mut tree = String::new();
        let mut count = 0;

        for window in windows {
            if window.is_minimized().unwrap_or(false) {
                continue;
            }
            
            count += 1;
            let title = window.title().unwrap_or_else(|_| "Unknown".to_string());
            let app_name = window.app_name().unwrap_or_else(|_| "Unknown".to_string());
            let x = window.x().unwrap_or(0);
            let y = window.y().unwrap_or(0);
            let w = window.width().unwrap_or(0);
            let h = window.height().unwrap_or(0);
            
            // Format: [n1] "Title" (App) at (x,y) size wxh
            tree.push_str(&format!(
                "[n{}] \"{}\" ({}) at ({},{}) size {}x{}\n",
                count, title, app_name, x, y, w, h
            ));
        }

        Ok(tree.trim().to_string())
    })();

    match result {
        Ok(tree) => {
            log::info!("do_get_screen_info: success, found tree with {} chars", tree.len());
            ToolResult {
                success: true,
                data: Some(json!({
                    "tree": tree
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_get_screen_info: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Find windows matching the given text and return their metadata as JSON nodes.
pub fn do_find_node_info(text: String) -> ToolResult {
    log::info!("do_find_node_info: text='{}'", text);
    
    if text.trim().is_empty() {
        return ToolResult {
            success: false,
            data: None,
            error: Some("text parameter must not be empty".into()),
        };
    }

    let result = (|| -> Result<serde_json::Value, String> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;
        let text_lower = text.to_lowercase();
        
        let mut nodes = Vec::new();
        let mut index = 0;

        for window in windows {
            if window.is_minimized().unwrap_or(false) {
                continue;
            }
            
            let title = window.title().unwrap_or_else(|_| "Unknown".to_string());
            let app_name = window.app_name().unwrap_or_else(|_| "Unknown".to_string());
            
            if title.to_lowercase().contains(&text_lower) || app_name.to_lowercase().contains(&text_lower) {
                let x = window.x().unwrap_or(0);
                let y = window.y().unwrap_or(0);
                let w = window.width().unwrap_or(0);
                let h = window.height().unwrap_or(0);
                
                nodes.push(json!({
                    "index": index,
                    "className": "Window",
                    "text": title,
                    "app_name": app_name,
                    "bounds": format!("[{},{}][{},{}]", x, y, x + w as i32, y + h as i32),
                    "clickable": true,
                }));
                index += 1;
            }
        }

        Ok(json!(nodes))
    })();

    match result {
        Ok(nodes) => {
            log::info!("do_find_node_info: success, found {} nodes", nodes.as_array().map_or(0, |a| a.len()));
            ToolResult {
                success: true,
                data: Some(json!({
                    "nodes": nodes
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_find_node_info: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Enumerate all currently running applications by checking open windows.
pub fn do_get_installed_apps(filter: Option<&str>) -> ToolResult {
    log::info!("do_get_installed_apps: filter={:?}", filter);

    let result = (|| -> Result<Vec<serde_json::Value>, String> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;
        let filter_lower = filter.map(|f| f.to_lowercase());

        let mut apps = std::collections::HashSet::new();
        let mut app_list = Vec::new();

        for window in windows {
            let app_name = window.app_name().unwrap_or_else(|_| "Unknown".to_string());
            if app_name == "Unknown" || app_name.is_empty() {
                continue;
            }

            if let Some(ref f) = filter_lower {
                if !app_name.to_lowercase().contains(f) {
                    continue;
                }
            }

            if apps.insert(app_name.clone()) {
                app_list.push(json!({
                    "app_name": app_name,
                    "package_name": app_name, // On desktop, we use app name as package handle
                    "is_system": false
                }));
            }
        }

        Ok(app_list)
    })();

    match result {
        Ok(apps) => {
            log::info!("do_get_installed_apps: success, found {} apps", apps.len());
            ToolResult {
                success: true,
                data: Some(json!({
                    "apps": apps
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_get_installed_apps: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}
