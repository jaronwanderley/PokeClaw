# PokeClaw Tool Reference

All 28 tools available to the agent loop, with parameter schemas, return formats, and platform support.

> **Legend:** Platform column — ✅ = all platforms (common), 📱 = mobile-only

---

## Common Tools (20)

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
