//! Desktop-only real OS input automation via enigo.
//!
//! Provides mouse (tap, swipe, long_press) and keyboard (input_text,
//! system_key) functions that simulate real OS-level input events.
//! Each function returns a `ToolResult` matching the plugin's observation
//! tool contract.

use crate::ToolResult;
use enigo::{
    Button, Coordinate,
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use std::thread;
use std::time::Duration;

/// Simulate a mouse click (tap) at absolute screen coordinates (x, y).
pub fn do_tap(x: i32, y: i32) -> ToolResult {
    log::info!("do_tap: x={}, y={}", x, y);
    let result = (|| -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create Enigo: {}", e))?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| format!("move_mouse failed: {}", e))?;
        enigo
            .button(Button::Left, Click)
            .map_err(|e| format!("button click failed: {}", e))?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info!("do_tap: success at ({}, {})", x, y);
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": format!("Tapped at ({}, {})", x, y)
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_tap: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Simulate a swipe gesture from (start_x, start_y) to (end_x, end_y)
/// by interpolating N mouse move steps over the given duration.
/// Defaults to 300ms / 10 steps if duration_ms is not specified.
pub fn do_swipe(
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    duration_ms: Option<i32>,
) -> ToolResult {
    let duration_ms = duration_ms.unwrap_or(300);
    let steps = 10;
    log::info!(
        "do_swipe: ({},{}) -> ({},{}) duration={}ms steps={}",
        start_x, start_y, end_x, end_y, duration_ms, steps
    );

    let result = (|| -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create Enigo: {}", e))?;

        // Move to start position
        enigo
            .move_mouse(start_x, start_y, Coordinate::Abs)
            .map_err(|e| format!("move_mouse to start failed: {}", e))?;

        // Press left button
        enigo
            .button(Button::Left, Press)
            .map_err(|e| format!("button press failed: {}", e))?;

        // Interpolate N steps from start to end
        let step_delay = Duration::from_millis(duration_ms as u64 / steps as u64);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let cx = (start_x as f64 + (end_x - start_x) as f64 * t).round() as i32;
            let cy = (start_y as f64 + (end_y - start_y) as f64 * t).round() as i32;
            enigo
                .move_mouse(cx, cy, Coordinate::Abs)
                .map_err(|e| format!("move_mouse step {} failed: {}", i, e))?;
            if i < steps {
                thread::sleep(step_delay);
            }
        }

        // Release left button
        enigo
            .button(Button::Left, Release)
            .map_err(|e| format!("button release failed: {}", e))?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info!(
                "do_swipe: success ({},{}) -> ({},{}) in {}ms",
                start_x, start_y, end_x, end_y, duration_ms
            );
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": format!(
                        "Swiped from ({},{}) to ({},{}) in {}ms",
                        start_x, start_y, end_x, end_y, duration_ms
                    )
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_swipe: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Simulate a long press at (x, y) by pressing, sleeping for duration_ms,
/// then releasing. Defaults to 500ms if duration_ms is not specified.
pub fn do_long_press(x: i32, y: i32, duration_ms: Option<i32>) -> ToolResult {
    let duration_ms = duration_ms.unwrap_or(500);
    log::info!("do_long_press: ({},{}) duration={}ms", x, y, duration_ms);

    let result = (|| -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create Enigo: {}", e))?;

        // Move to target position
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| format!("move_mouse failed: {}", e))?;

        // Press left button
        enigo
            .button(Button::Left, Press)
            .map_err(|e| format!("button press failed: {}", e))?;

        // Hold for duration
        thread::sleep(Duration::from_millis(duration_ms as u64));

        // Release left button
        enigo
            .button(Button::Left, Release)
            .map_err(|e| format!("button release failed: {}", e))?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info!("do_long_press: success at ({},{}) for {}ms", x, y, duration_ms);
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": format!("Long pressed at ({},{}) for {}ms", x, y, duration_ms)
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_long_press: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Type text into the currently focused window.
///
/// If `clear_first` is true, sends Ctrl+A followed by Backspace before
/// typing. The `node_id` parameter is ignored on desktop (text is typed
/// into whatever window has focus).
pub fn do_input_text(text: &str, _node_id: Option<&str>, clear_first: Option<bool>) -> ToolResult {
    log::info!(
        "do_input_text: text='{}' ({} chars), clear_first={:?}",
        text,
        text.len(),
        clear_first
    );

    if text.trim().is_empty() {
        return ToolResult {
            success: false,
            data: None,
            error: Some("text parameter must not be empty".into()),
        };
    }

    let result = (|| -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create Enigo: {}", e))?;

        if clear_first.unwrap_or(false) {
            // Select all: Ctrl+A
            enigo
                .key(Key::Control, Press)
                .map_err(|e| format!("Ctrl press failed: {}", e))?;
            enigo
                .key(Key::Unicode('a'), Click)
                .map_err(|e| format!("Ctrl+A click failed: {}", e))?;
            enigo
                .key(Key::Control, Release)
                .map_err(|e| format!("Ctrl release failed: {}", e))?;

            // Small delay for the selection to register
            thread::sleep(Duration::from_millis(50));

            // Delete selection
            enigo
                .key(Key::Backspace, Click)
                .map_err(|e| format!("Backspace failed: {}", e))?;

            thread::sleep(Duration::from_millis(50));
        }

        // Type the text
        enigo
            .text(text)
            .map_err(|e| format!("text() failed: {}", e))?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info!("do_input_text: success — typed {} chars", text.len());
            ToolResult {
                success: true,
                data: Some(serde_json::json!({
                    "message": format!("Input text: '{}' (clear_first={})", text, clear_first.unwrap_or(false))
                })),
                error: None,
            }
        }
        Err(e) => {
            log::error!("do_input_text: failed — {}", e);
            ToolResult {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Press a system key mapped from an action string.
///
/// Maps common Android navigation actions to desktop keyboard equivalents:
/// - back → Escape
/// - home → Meta (Windows/Super key)
/// - enter → Return
/// - escape → Escape
/// - delete → Backspace
/// - tab → Tab
/// - volume_up → VolumeUp
/// - volume_down → VolumeDown
///
/// Returns an error for unknown action strings.
pub fn do_system_key(action: &str) -> ToolResult {
    log::info!("do_system_key: action='{}'", action);

    let key = match action.to_lowercase().as_str() {
        "back" => Some(Key::Escape),
        "home" => Some(Key::Meta),
        "enter" => Some(Key::Return),
        "escape" => Some(Key::Escape),
        "delete" => Some(Key::Backspace),
        "tab" => Some(Key::Tab),
        "volume_up" => Some(Key::VolumeUp),
        "volume_down" => Some(Key::VolumeDown),
        _ => None,
    };

    match key {
        Some(k) => {
            let result = (|| -> Result<(), String> {
                let mut enigo = Enigo::new(&Settings::default())
                    .map_err(|e| format!("Failed to create Enigo: {}", e))?;
                enigo
                    .key(k, Click)
                    .map_err(|e| format!("key click failed: {}", e))?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    log::info!("do_system_key: success — action='{}'", action);
                    ToolResult {
                        success: true,
                        data: Some(serde_json::json!({
                            "message": format!("Pressed system key: {}", action.to_lowercase()),
                            "action": action.to_lowercase(),
                        })),
                        error: None,
                    }
                }
                Err(e) => {
                    log::error!("do_system_key: failed — {}", e);
                    ToolResult {
                        success: false,
                        data: None,
                        error: Some(e),
                    }
                }
            }
        }
        None => {
            let supported = "back, home, enter, escape, delete, tab, volume_up, volume_down";
            let msg = format!(
                "Unknown system key action '{}'. Supported: {}",
                action, supported
            );
            log::warn!("do_system_key: {}", msg);
            ToolResult {
                success: false,
                data: None,
                error: Some(msg),
            }
        }
    }
}
