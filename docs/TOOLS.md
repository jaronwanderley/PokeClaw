# PitoAgent Tool Reference

All 28 tools available to the agent loop, with parameter schemas, return formats, and platform support.

> **Legend:** ✅ = real implementation, 🔁 = equivalent capability (different backend per platform), 🖥️ = mock (dev only), ❌ = no equivalent

---
---

## Android Tools (38)

Full phone control via AccessibilityService + NotificationListener + hardware APIs.

| Category | Tools | Count |
|----------|-------|:-----:|
| **Screen** | `get_screen_info` `find_node_info` `take_screenshot` | 3 |
| **Touch** | `tap` `tap_node` `long_press` `swipe` `scroll_to_find` `find_and_tap` | 6 |
| **Input** | `input_text` `system_key` | 2 |
| **Apps** | `open_app` `get_installed_apps` | 2 |
| **Communication** | `send_message` `send_file` `auto_reply` `make_call` | 4 |
| **Device** | `get_device_info` `get_notifications` `clipboard` | 3 |
| **Hardware** | `toggle_flashlight` `toggle_bluetooth` `scan_bluetooth` `get_sensors` | 4 |
| **Alerts** | `show_notification` `vibrate` `play_sound` | 3 |
| **Scheduling** | `schedule_task` `set_timer` | 2 |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` | 5 |
| **Utility** | `wait` `repeat_actions` `finish` | 3 |

### Android Hardware Tools

#### `toggle_flashlight`
```
Parameters:
  state (string, required) — "on" or "off"
Returns: { success: bool }
Notes: Uses CameraManager.setTorchMode(). Requires camera permission on some devices.
```

#### `toggle_bluetooth`
```
Parameters:
  state (string, required) — "on" or "off"
Returns: { success: bool, data: { state: string } }
Notes: Uses BluetoothAdapter.enable()/disable(). Requires BLUETOOTH_ADMIN permission.
```

#### `scan_bluetooth`
```
Parameters:
  duration_seconds (integer, optional) — How long to scan in seconds (default: 5, max: 30)
  filter_name      (string,  optional) — Only return devices whose name contains this string
  filter_rssi_min  (integer, optional) — Minimum RSSI to include (default: -100, i.e. all)
Returns: { success: bool, data: {
  devices: [
    {
      name: "Mom's Watch",
      address: "AA:BB:CC:DD:EE:FF",
      rssi: -55,
      distance_m: 3.2,
      type: "wearable",
      last_seen: "2026-04-13T18:00:00Z"
    },
    ...
  ],
  scan_duration_ms: 5023,
  count: 5
} }
Notes: Uses BluetoothLeScanner (API 21+). Distance is estimated from RSSI using
       the log-distance path loss model: d = 10^((txPower - RSSI) / (10 * N))
       where N = 2.0 (free space) to 4.0 (obstructed). The estimate is approximate —
       treat it as "immediate" (< 1m), "near" (1-3m), "far" (3-10m), or "very far" (> 10m).
       Requires BLUETOOTH_SCAN and BLUETOOTH_CONNECT permissions (Android 12+) or
       ACCESS_FINE_LOCATION (Android 11 and below).
       Useful for: "is my watch nearby?", "find my earbuds", presence detection,
       iBeacon/Eddystone proximity, smart home device discovery.
```

#### `get_sensors`
```
Parameters:
  sensors (string, optional) — Comma-separated list: "accelerometer,gyroscope,proximity,light,battery"
                              (default: all available)
Returns: { success: bool, data: {
  accelerometer: { x, y, z },
  gyroscope: { x, y, z },
  proximity: { near: bool, distance: float },
  light: { lux: float },
  battery: { level, temperature, charging, health }
} }
Notes: Reads SensorManager + BatteryManager. Values are instantaneous snapshots.
```

### Android Scheduling Tools

#### `schedule_task`
```
Parameters:
  prompt     (string,  required) — Task description for the agent (e.g. "Send birthday message to Mom")
  trigger_at (string,  required) — ISO 8601 datetime or relative time (e.g. "2026-04-13T18:00:00" or "in 2 hours")
  repeat     (string,  optional) — "once" (default), "daily", "weekly", "monthly"
Returns: { success: bool, data: { task_id: string, trigger_at: string } }
Notes: Uses AlarmManager (exact) or WorkManager (flexible). When the alarm fires,
       the app wakes and starts the agent loop with the given prompt.
       This enables "remind Mom at 6pm" → agent fires at 18:00 and executes the full task.
```

#### `set_timer`
```
Parameters:
  duration_seconds (integer, required) — Timer duration in seconds
  label            (string,  optional) — What the timer is for
Returns: { success: bool, data: { timer_id: string, fires_at: string } }
Notes: Simple countdown. Fires a notification when done. Does NOT trigger the agent loop.
```

### Android Alert Tools

#### `show_notification`
```
Parameters:
  title   (string,  required) — Notification title
  body    (string,  required) — Notification body text
  sound   (boolean, optional) — Play notification sound (default: true)
  actions (string,  optional) — Comma-separated action buttons (e.g. "Open,Dismiss")
Returns: { success: bool, data: { notification_id: string } }
Notes: Uses NotificationManager with a foreground-compatible channel.
       The notification appears in the system notification shade.
       Action button taps can be routed back to the agent.
```

#### `vibrate`
```
Parameters:
  pattern (string, optional) — Vibration pattern: "short" (200ms), "long" (1s),
                               "double" (200ms pause 100ms 200ms), "sos",
                               or custom millis "200,100,200,100,500" (default: "short")
  repeat  (int,    optional) — How many times to repeat the pattern (default: 1)
Returns: { success: bool }
Notes: Uses Vibrator service. Requires VIBRATE permission.
       "sos" plays ... --- ... (3 short, 3 long, 3 short).
```

#### `play_sound`
```
Parameters:
  type  (string, optional) — "notification" (default), "alarm", "ringtone",
                             "beep", or "tts" (text-to-speech)
  text  (string, optional) — Text to speak (required when type is "tts")
  volume (float, optional) — Volume 0.0–1.0 (default: 0.7)
Returns: { success: bool }
Notes: Uses MediaPlayer for sounds, TextToSpeech for TTS.
       "notification"/"alarm"/"ringtone" plays the system default sound for that type.
       "beep" plays a short programmatically-generated tone.
       "tts" speaks the given text aloud — useful for the agent to alert the user
       without looking at the screen (e.g. "Mom just messaged you").
```

## Desktop Tools (planned — 34)

Real OS automation via mouse/keyboard + system notifications + scheduling + audio.

| Category | Tool | What | Backend |
|----------|------|------|---------|
| **Screen** | `get_screen_info` | Screen size, resolution, UI tree | UI Automation (Win32) / AXUIElement (macOS) / AT-SPI (Linux) |
| | `find_node_info` | Find UI element by text/ID/role | Same as above |
| | `take_screenshot` | Capture screen as PNG | xcap / CGWindowList / BitBlt |
| **Mouse** | `click` | Left click at coordinates | enigo / CGEvent / SendInput |
| | `double_click` | Double left click | enigo (double click) |
| | `right_click` | Right click (context menu) | enigo (right click) |
| | `move_mouse` | Move cursor to coordinates | enigo / CGEvent |
| | `drag` | Click and drag between points | enigo (press → move → release) |
| | `scroll` | Mouse wheel scroll (up/down, N clicks) | enigo (scroll) |
| **Keyboard** | `type_text` | Type a string into focused field | enigo / CGEvent |
| | `press_key` | Press a single key (Enter, Tab, Esc, F1...) | enigo / CGEvent |
| | `hotkey` | Press key combo (Ctrl+C, Cmd+Q, Alt+Tab) | enigo (multiple keys) |
| **Apps** | `open_app` | Launch app by name or path | `std::process::Command` |
| | `get_installed_apps` | List installed applications | registry / `/Applications` / `.desktop` |
| | `switch_app` | Bring app to foreground (Alt+Tab / Cmd+Tab) | enigo hotkey + focus |
| **Communication** | `send_email` | Open default mail client with composed email | `mailto:` URI |
| **Device** | `get_device_info` | OS, CPU, RAM, disk, network | sysinfo crate |
| | `clipboard` | Read/write system clipboard | arboard crate |
| | `get_sensors` | CPU temp, fan speed, disk health | sysinfo crate |
| **Notifications** | `show_notification` | Display system notification | Win32 toast / NSUserNotification / libnotify |
| **Bluetooth** | `scan_bluetooth` | Scan nearby BLE devices | btleplug (cross-platform) |
| **Audio** | `play_sound` | Play audio, beep, or TTS | rodio / cpal |
| **Scheduling** | `schedule_task` | Schedule a future agent task | cron / Task Scheduler / launchd |
| | `set_timer` | Countdown timer, notifies when done | pure logic + `show_notification` |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` | File I/O | App data dir |
| **Utility** | `wait` `repeat_actions` `finish` | Pure logic | — |

### Desktop key Rust crates

| Crate | Purpose | Platforms |
|-------|---------|-----------|
| [enigo](https://crates.io/crates/enigo) | Mouse + keyboard input | Windows, macOS, Linux |
| [xcap](https://crates.io/crates/xcap) | Screen capture | Windows, macOS, Linux |
| [arboard](https://crates.io/crates/arboard) | Clipboard read/write | Windows, macOS, Linux |
| [sysinfo](https://crates.io/crates/sysinfo) | System info + sensors | Windows, macOS, Linux |
| [rodio](https://crates.io/crates/rodio) | Audio playback + TTS | Windows, macOS, Linux |
| [btleplug](https://crates.io/crates/btleplug) | BLE scan (nearby devices, RSSI) | Windows, macOS, Linux |
| [windows-rs](https://crates.io/crates/windows) | Win32 UI Automation + toast | Windows |
| [cocoa](https://crates.io/crates/cocoa) | macOS AXUIElement + notifications | macOS |
| [atspi](https://crates.io/crates/atspi) | Linux Accessibility (AT-SPI) | Linux |

> Desktop tools are planned for M006. Currently uses Android tool mocks for development.

## iOS Tools (planned — 19)

Heavily sandboxed, but CoreBluetooth scanning, CoreMotion sensors, local notifications, audio playback, TTS, and flashlight all work in production apps.

| Category | Tool | What | Scope |
|----------|------|------|-------|
| **Device** | `get_device_info` | Model, OS, memory | `UIDevice` / `ProcessInfo` |
| **Sensors** | `get_sensors` | Accelerometer, gyroscope, magnetometer | `CoreMotion` (CMPedometer / CMMotionManager) |
| **Notifications** | `show_notification` | Local push notification to user | `UNUserNotificationCenter` (user must grant permission) |
| **Audio** | `play_sound` | Play sounds or speak text aloud | `AVAudioPlayer` (sounds) + `AVSpeechSynthesizer` (TTS) |
| **Bluetooth** | `scan_bluetooth` | Scan nearby BLE devices, RSSI, distance | `CoreBluetooth` CBCentralManager (no special entitlement) |
| **Hardware** | `toggle_flashlight` | Turn flashlight on/off | `AVCaptureDevice.setTorchMode` (works in production!) |
| **Clipboard** | `clipboard` | Read/write clipboard | `UIPasteboard` (own app) |
| **Deep Links** | `open_app` | Open app via URL scheme | `message://`, `tel://`, etc. |
| | `make_call` | Open Phone dialer | `tel://` (user confirms) |
| | `send_message` | Open Messages app | `message://` (user types) |
| **Timers** | `set_timer` | Local countdown, fires notification | In-process timer |
| **Knowledge Base** | `kb_write` `kb_read` `kb_search` `kb_append` `kb_add_todo` | File I/O | App sandbox |
| **Utility** | `wait` `finish` | Pure logic | — |

### iOS sensor access

CoreMotion is available in production apps without special entitlements:

| Sensor | API | What you get |
|--------|-----|-------------|
| Accelerometer | `CMAccelerometerData` | x, y, z acceleration (G) |
| Gyroscope | `CMGyroData` | x, y, z rotation rate (rad/s) |
| Magnetometer | `CMMagnetometerData` | x, y, z magnetic field (µT) |
| Device motion | `CMDeviceMotion` | attitude (pitch/roll/yaw), gravity, user acceleration |
| Pedometer | `CMPedometer` | step count, distance, stairs climbed |

### iOS audio + flashlight + bluetooth

| Capability | API | Works in production? |
|-----------|-----|---------------------|
| Play sound files | `AVAudioPlayer` | Yes — play bundled or downloaded audio |
| Text to speech | `AVSpeechSynthesizer` | Yes — speak any text, multiple languages |
| Flashlight on/off | `AVCaptureDevice.setTorchMode` | Yes — no special entitlement needed |
| Scan BLE devices | `CBCentralManager.scanForPeripherals` | Yes — name, RSSI, service UUID, no special entitlement |
| Vibrate | `UIImpactFeedbackGenerator` / `AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)` | Yes — haptic feedback only (short tap, no patterns) |

> CoreBluetooth works in production iOS apps without any special entitlements. The app can discover nearby BLE devices, read their RSSI (signal strength), and estimate distance. This enables beacon tracking, "find my device" features, and proximity-based automation.
> No Bluetooth *control* (pair/connect/write) without the device implementing a specific BLE service the app knows about. `scan_bluetooth` is read-only observation.
> No custom vibration patterns on iOS — only short haptic taps via UIImpactFeedbackGenerator.

### iOS limitations

A production App Store app on iOS **cannot**:

- Read the UI tree of other apps
- Simulate taps, swipes, or gestures outside its own process
- Take screenshots of other apps or the system
- Listen to system-wide or other apps' notifications
- Enumerate installed apps
- Control Bluetooth, flashlight, or other hardware directly
- Schedule tasks that execute when the app is not running (only background fetch with strict limits)
- Access the clipboard of other apps (only general pasteboard, and only when foreground)

XCUITest / XCTest APIs are only available inside the test runner — they cannot be used for runtime automation in a shipped app.

## Shared Tools (all platforms)

These 7 tools are pure logic (file I/O or computation) and work identically on every platform:

`kb_write` · `kb_read` · `kb_search` · `kb_append` · `kb_add_todo` · `wait` · `finish`

---

## Detailed Schemas

### Common Tools (20)

| # | Name | Description | Platform |
|---|------|-------------|----------|
| 1 | `get_screen_info` | Get current screen info: size, orientation, UI hierarchy | ✅ |
| 2 | `find_node_info` | Find a UI node by text, resource ID, or description | ✅ |
| 3 | `input_text` | Type text into the currently focused input field | ✅ |
| 4 | `system_key` | Press a system key (back, home, recent, enter, etc.) | ✅ |
| 5 | `open_app` | Open an application by package name or display name | ✅ |
| 6 | `get_installed_apps` | List installed applications with package and display names | ✅ |
| 7 | `take_screenshot` | Capture current screen as base64-encoded PNG | ✅ |
| 8 | `wait` | Wait a specified number of milliseconds | ✅ |
| 9 | `repeat_actions` | Repeat a sequence of actions multiple times with delay | ✅ |
| 10 | `clipboard` | Read from or write to the system clipboard | ✅ |
| 11 | `send_file` | Send a file to a contact via a messaging app | ✅ |
| 12 | `get_device_info` | Get device info: model, OS version, screen size, battery, network | ✅ |
| 13 | `get_notifications` | Get recent notifications from the notification bar | ✅ |
| 14 | `make_call` | Initiate a phone call to a number or contact | ✅ |
| 15 | `finish` | Signal task completion with result summary | ✅ |
| 16 | `kb_write` | Write or create a note in the knowledge base vault | ✅ |
| 17 | `kb_read` | Read a note from the knowledge base by path | ✅ |
| 18 | `kb_search` | Full-text search across all knowledge base notes | ✅ |
| 19 | `kb_append` | Append content to an existing knowledge base note | ✅ |
| 20 | `kb_add_todo` | Add a todo item to today's todo list | ✅ |

## Mobile-Only Tools (8)

| # | Name | Description | Platform |
|---|------|-------------|----------|
| 21 | `tap` | Tap on screen at specified coordinates | 📱 |
| 22 | `tap_node` | Tap on a UI node found by text/resource ID/description | 📱 |
| 23 | `long_press` | Long press at coordinates for context menus | 📱 |
| 24 | `swipe` | Swipe from one coordinate to another | 📱 |
| 25 | `scroll_to_find` | Scroll through a container to find specific text | 📱 |
| 26 | `find_and_tap` | Find a UI element by text and tap it (with auto-scroll) | 📱 |
| 27 | `send_message` | Send a text message to a contact via a messaging app | 📱 |
| 28 | `auto_reply` | Auto-reply to an incoming message notification | 📱 |

---

## Detailed Parameter Schemas

### 1. `get_screen_info`
```
Parameters: (none)
Returns: { success: bool, data: { width, height, orientation, ui_hierarchy } }
```

### 2. `find_node_info`
```
Parameters:
  text        (string,  optional) — Text to search for in the UI node
  resource_id (string,  optional) — Resource ID to match (e.g. 'com.android:id/button')
  description (string,  optional) — Content description to match
  index       (integer, optional) — Index of matching node when multiple exist (0-based)
Returns: { success: bool, data: { bounds, text, clickable, ... } }
```

### 3. `input_text`
```
Parameters:
  text (string, required) — Text to type into the focused input field
Returns: { success: bool, data: { typed: string } }
```

### 4. `system_key`
```
Parameters:
  key (string, required) — Key to press: back | home | recent | enter | delete | tab | escape | volume_up | volume_down
Returns: { success: bool }
```

### 5. `open_app`
```
Parameters:
  app_name (string, required) — App package name (e.g. 'com.android.chrome') or display name (e.g. 'Chrome')
Returns: { success: bool, data: { package: string } }
```

### 6. `get_installed_apps`
```
Parameters: (none)
Returns: { success: bool, data: [{ package_name, display_name }] }
```

### 7. `take_screenshot`
```
Parameters: (none)
Returns: { success: bool, data: { image_base64: string, format: "png" } }
```

### 8. `wait`
```
Parameters:
  milliseconds (integer, required) — Time to wait in milliseconds
Returns: { success: bool, data: { waited_ms: integer } }
```

### 9. `repeat_actions`
```
Parameters:
  actions  (string,  required) — JSON array of action objects, e.g. [{"tool":"tap","params":{"x":500,"y":800}}]
  count    (integer, required) — Number of times to repeat the action sequence
  delay_ms (integer, optional) — Delay between iterations in ms (default: 1000)
Returns: { success: bool, data: { iterations: integer } }
```

### 10. `clipboard`
```
Parameters:
  action (string, required)          — 'read' or 'write'
  text   (string, conditionally req) — Text to write (required when action is 'write')
Returns: { success: bool, data: { text: string } | { written: bool } }
```

### 11. `send_file`
```
Parameters:
  contact   (string, required) — Contact name or phone number
  file_path (string, required) — Path to the file to send
  app       (string, required) — Messaging app: 'whatsapp', 'telegram', 'line', etc.
Returns: { success: bool }
```

### 12. `get_device_info`
```
Parameters: (none)
Returns: { success: bool, data: { model, os_version, screen_width, screen_height, battery_level, network_status } }
```

### 13. `get_notifications`
```
Parameters:
  limit (integer, optional) — Maximum notifications to return (default: 10)
Returns: { success: bool, data: [{ app, text, timestamp }] }
```

### 14. `make_call`
```
Parameters:
  contact (string, required) — Phone number or contact name to call
Returns: { success: bool }
```

### 15. `finish`
```
Parameters:
  result  (string,  optional) — Summary of what was accomplished
  success (boolean, optional) — Whether task completed successfully (default: true)
Returns: { success: bool, result: string }
```

### 16. `kb_write`
```
Parameters:
  path    (string,  required) — File path relative to vault root, e.g. 'notes/meeting.md'
  content (string,  required) — Markdown content to write (frontmatter added automatically)
  type    (string,  optional) — Note type: note | todo | calendar | journal | research (default: note)
  date    (string,  optional) — Date in YYYY-MM-DD format (default: today)
  tags    (string,  optional) — Comma-separated tags, e.g. 'work,meeting,q2'
Returns: { success: bool, data: { path: string } }
```

### 17. `kb_read`
```
Parameters:
  path (string, required) — File path relative to vault root
Returns: { success: bool, data: { content: string, path: string } }
```

### 18. `kb_search`
```
Parameters:
  query (string, required) — Search query. Case-insensitive, searches all .md files
Returns: { success: bool, data: [{ path, snippet, score }] }
```

### 19. `kb_append`
```
Parameters:
  path    (string, required) — File path relative to vault root
  content (string, required) — Markdown content to append
Returns: { success: bool, data: { path: string } }
```

### 20. `kb_add_todo`
```
Parameters:
  text     (string,  required) — The todo item text
  due      (string,  optional) — Due date in YYYY-MM-DD format
  priority (string,  optional) — Priority: high | medium | low
Returns: { success: bool, data: { path: string } }
```

### 21. `tap` 📱
```
Parameters:
  x (integer, required) — X coordinate to tap
  y (integer, required) — Y coordinate to tap
Returns: { success: bool }
```

### 22. `tap_node` 📱
```
Parameters:
  text        (string,  optional) — Text to search for
  resource_id (string,  optional) — Resource ID to match
  description (string,  optional) — Content description to match
  index       (integer, optional) — Index of matching node (0-based)
Returns: { success: bool }
```

### 23. `long_press` 📱
```
Parameters:
  x            (integer, required) — X coordinate
  y            (integer, required) — Y coordinate
  duration_ms  (integer, optional) — Duration in ms (default: 500)
Returns: { success: bool }
```

### 24. `swipe` 📱
```
Parameters:
  start_x     (integer, required) — Starting X coordinate
  start_y     (integer, required) — Starting Y coordinate
  end_x       (integer, required) — Ending X coordinate
  end_y       (integer, required) — Ending Y coordinate
  duration_ms (integer, optional) — Duration in ms (default: 300)
Returns: { success: bool }
```

### 25. `scroll_to_find` 📱
```
Parameters:
  text       (string,  optional) — Text to search for while scrolling
  direction  (string,  optional) — 'up' or 'down' (default: 'down')
  max_scrolls (integer, optional) — Maximum scroll iterations (default: 10)
Returns: { success: bool, data: { found: bool, text: string } }
```

### 26. `find_and_tap` 📱
```
Parameters:
  text   (string,  required) — Text of the element to find and tap
  scroll (boolean, optional) — Whether to scroll to find (default: true)
Returns: { success: bool }
```

### 27. `send_message` 📱
```
Parameters:
  contact (string, required) — Contact name or phone number
  message (string, required) — Message text to send
  app     (string, required) — Messaging app: 'whatsapp', 'telegram', 'line', etc.
Returns: { success: bool }
```

### 28. `auto_reply` 📱
```
Parameters:
  notification_id (string,  required) — ID of the notification to reply to
  reply_text      (string,  optional) — Reply text (if omitted, generates automatic reply)
Returns: { success: bool, data: { reply: string } }
```
