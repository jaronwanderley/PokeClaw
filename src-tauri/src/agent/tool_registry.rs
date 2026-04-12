// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

use std::collections::HashMap;
use log::info;
use serde_json::{json, Map, Value};

/// Parameter type for tool definitions.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    String,
    Integer,
    Number,
    Boolean,
}

impl ParamType {
    /// Returns the JSON Schema type string.
    fn schema_type(&self) -> &str {
        match self {
            ParamType::String => "string",
            ParamType::Integer => "integer",
            ParamType::Number => "number",
            ParamType::Boolean => "boolean",
        }
    }
}

/// A single parameter for a tool, mirroring Kotlin `ToolParameter`.
#[derive(Debug, Clone)]
pub struct ToolParam {
    pub name: String,
    pub param_type: ParamType,
    pub description: String,
    pub is_required: bool,
}

impl ToolParam {
    /// Creates a new ToolParam.
    pub fn new(name: &str, param_type: ParamType, description: &str, is_required: bool) -> Self {
        Self {
            name: name.to_string(),
            param_type,
            description: description.to_string(),
            is_required,
        }
    }

    /// Converts this parameter into a JSON Schema property value.
    /// Returns a serde_json::Value like: `{"type": "string", "description": "..."}`
    pub fn to_schema_property(&self) -> Value {
        json!({
            "type": self.param_type.schema_type(),
            "description": self.description
        })
    }
}

/// A tool specification, mirroring Kotlin `BaseTool`.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description_en: String,
    pub description_cn: String,
    pub parameters: Vec<ToolParam>,
    /// Whether this tool is mobile-only (vs common across all platforms).
    pub is_mobile: bool,
}

impl ToolSpec {
    /// Creates a new ToolSpec.
    pub fn new(
        name: &str,
        description_en: &str,
        description_cn: &str,
        parameters: Vec<ToolParam>,
        is_mobile: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            description_en: description_en.to_string(),
            description_cn: description_cn.to_string(),
            parameters,
            is_mobile,
        }
    }

    /// Generates an OpenAI-compatible JSON Schema object for this tool's parameters.
    ///
    /// Returns a Value like:
    /// ```json
    /// {
    ///   "type": "object",
    ///   "properties": { "x": { "type": "integer", "description": "..." } },
    ///   "required": ["x"]
    /// }
    /// ```
    pub fn parameters_json_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();

        for param in &self.parameters {
            properties.insert(param.name.clone(), param.to_schema_property());
            if param.is_required {
                required.push(Value::String(param.name.clone()));
            }
        }

        json!({
            "type": "object",
            "properties": Value::Object(properties),
            "required": required,
        })
    }

    /// Returns the description in the specified language.
    /// `true` = Chinese, `false` = English.
    pub fn description(&self, use_chinese: bool) -> &str {
        if use_chinese {
            &self.description_cn
        } else {
            &self.description_en
        }
    }
}

/// Registry of all available tools, mirroring Kotlin `ToolRegistry`.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolSpec>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };
        registry.register_common_tools();
        registry.register_mobile_tools();
        info!(
            "ToolRegistry initialized with {} tools ({} common + {} mobile)",
            registry.tools.len(),
            registry.common_tools().len(),
            registry.mobile_tools().len(),
        );
        registry
    }
}

impl ToolRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Registers a tool spec.
    pub fn register(&mut self, spec: ToolSpec) {
        self.tools.insert(spec.name.clone(), spec);
    }

    /// Gets a tool spec by name, logging the lookup.
    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        let result = self.tools.get(name);
        if result.is_some() {
            info!("ToolRegistry lookup: '{}' found", name);
        } else {
            info!("ToolRegistry lookup: '{}' not found", name);
        }
        result
    }

    /// Returns all registered tool specs.
    pub fn all_specs(&self) -> Vec<&ToolSpec> {
        self.tools.values().collect()
    }

    /// Returns specs for mobile-only tools (tap, swipe, etc.).
    pub fn mobile_tools(&self) -> Vec<&ToolSpec> {
        self.tools.values().filter(|t| t.is_mobile).collect()
    }

    /// Returns specs for tools shared across all platforms.
    pub fn common_tools(&self) -> Vec<&ToolSpec> {
        self.tools.values().filter(|t| !t.is_mobile).collect()
    }

    /// Returns the total number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Returns true if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Registers the 20 common tools (shared across TV and mobile).
    fn register_common_tools(&mut self) {
        // 1. get_screen_info
        self.register(ToolSpec::new(
            "get_screen_info",
            "Get the current screen information including size, orientation, and UI hierarchy (accessibility tree). Use this to understand what's on screen before taking action.",
            "獲取當前屏幕信息，包括尺寸、方向和 UI 層次結構（無障礙樹）。用於在執行操作前了解屏幕內容。",
            vec![],
            false,
        ));

        // 2. find_node_info
        self.register(ToolSpec::new(
            "find_node_info",
            "Find a specific UI node by text, content description, or resource ID. Returns the node's bounds, text, and clickable status.",
            "通過文本、內容描述或資源 ID 查找特定的 UI 節點。返回節點的邊界、文本和可點擊狀態。",
            vec![
                ToolParam::new("text", ParamType::String, "Text to search for in the UI node", false),
                ToolParam::new("resource_id", ParamType::String, "Resource ID to match (e.g. 'com.android:id/button')", false),
                ToolParam::new("description", ParamType::String, "Content description to match", false),
                ToolParam::new("index", ParamType::Integer, "Index of the matching node to select when multiple matches exist (0-based, default: 0)", false),
            ],
            false,
        ));

        // 3. input_text
        self.register(ToolSpec::new(
            "input_text",
            "Type text into the currently focused input field. Use this after tapping on a text field to enter content.",
            "在當前焦點的輸入框中輸入文本。在點擊文本框後使用此工具輸入內容。",
            vec![
                ToolParam::new("text", ParamType::String, "Text to type into the focused input field", true),
            ],
            false,
        ));

        // 4. system_key
        self.register(ToolSpec::new(
            "system_key",
            "Press a system key (back, home, recent apps, enter, etc.). Use for navigation that doesn't involve tapping screen coordinates.",
            "按下系統按鍵（返回、主頁、最近應用、確認鍵等）。用於不涉及點擊屏幕座標的導航操作。",
            vec![
                ToolParam::new("key", ParamType::String, "Key to press: back | home | recent | enter | delete | tab | escape | volume_up | volume_down", true),
            ],
            false,
        ));

        // 5. open_app
        self.register(ToolSpec::new(
            "open_app",
            "Open an application by its package name or display name. Returns success or error if the app is not found.",
            "通過包名或顯示名稱打開應用。如果找不到應用則返回錯誤。",
            vec![
                ToolParam::new("app_name", ParamType::String, "App package name (e.g. 'com.android.chrome') or display name (e.g. 'Chrome')", true),
            ],
            false,
        ));

        // 6. get_installed_apps
        self.register(ToolSpec::new(
            "get_installed_apps",
            "Get a list of installed applications on the device. Returns package names and display names.",
            "獲取設備上已安裝的應用列表。返回包名和顯示名稱。",
            vec![],
            false,
        ));

        // 7. take_screenshot
        self.register(ToolSpec::new(
            "take_screenshot",
            "Take a screenshot of the current screen. Returns the image as base64-encoded PNG.",
            "截取當前屏幕。返回 base64 編碼的 PNG 圖像。",
            vec![],
            false,
        ));

        // 8. wait
        self.register(ToolSpec::new(
            "wait",
            "Wait for a specified number of milliseconds. Use for page loads, animations, or transitions to complete.",
            "等待指定的毫秒數。用於等待頁面加載、動畫或過渡完成。",
            vec![
                ToolParam::new("milliseconds", ParamType::Integer, "Time to wait in milliseconds (e.g. 2000 for 2 seconds)", true),
            ],
            false,
        ));

        // 9. repeat_actions
        self.register(ToolSpec::new(
            "repeat_actions",
            "Repeat a sequence of actions multiple times with a delay between each iteration. Useful for scrolling through lists or performing repetitive tasks.",
            "重複執行一組動作多次，每次之間有延遲。適用於滾動列表或執行重複性任務。",
            vec![
                ToolParam::new("actions", ParamType::String, "JSON array of action objects to repeat, e.g. [{\"tool\":\"tap\",\"params\":{\"x\":500,\"y\":800}}]", true),
                ToolParam::new("count", ParamType::Integer, "Number of times to repeat the action sequence", true),
                ToolParam::new("delay_ms", ParamType::Integer, "Delay in milliseconds between each iteration (default: 1000)", false),
            ],
            false,
        ));

        // 10. clipboard
        self.register(ToolSpec::new(
            "clipboard",
            "Read from or write to the system clipboard. Use 'action' to specify read or write.",
            "讀取或寫入系統剪貼板。使用 'action' 指定讀取或寫入。",
            vec![
                ToolParam::new("action", ParamType::String, "Action to perform: 'read' or 'write'", true),
                ToolParam::new("text", ParamType::String, "Text to write (required when action is 'write')", false),
            ],
            false,
        ));

        // 11. send_file
        self.register(ToolSpec::new(
            "send_file",
            "Send a file to a contact via a specified messaging app.",
            "通過指定的消息應用向聯繫人發送文件。",
            vec![
                ToolParam::new("contact", ParamType::String, "Contact name or phone number to send the file to", true),
                ToolParam::new("file_path", ParamType::String, "Path to the file to send", true),
                ToolParam::new("app", ParamType::String, "Messaging app to use (e.g. 'whatsapp', 'telegram', 'line')", true),
            ],
            false,
        ));

        // 12. get_device_info
        self.register(ToolSpec::new(
            "get_device_info",
            "Get device information: model, OS version, screen size, battery level, and network status.",
            "獲取設備信息：型號、系統版本、屏幕尺寸、電池電量和網絡狀態。",
            vec![],
            false,
        ));

        // 13. get_notifications
        self.register(ToolSpec::new(
            "get_notifications",
            "Get recent notifications from the notification bar. Returns notification text, app, and timestamp.",
            "獲取通知欄的最近通知。返回通知文本、應用和時間戳。",
            vec![
                ToolParam::new("limit", ParamType::Integer, "Maximum number of notifications to return (default: 10)", false),
            ],
            false,
        ));

        // 14. make_call
        self.register(ToolSpec::new(
            "make_call",
            "Initiate a phone call to the specified number or contact.",
            "撥打電話給指定的號碼或聯繫人。",
            vec![
                ToolParam::new("contact", ParamType::String, "Phone number or contact name to call", true),
            ],
            false,
        ));

        // 15. finish
        self.register(ToolSpec::new(
            "finish",
            "Signal that the current task is complete. Call this when all requested actions have been performed successfully.",
            "表示當前任務已完成。當所有請求的操作都成功執行後調用此工具。",
            vec![
                ToolParam::new("result", ParamType::String, "Summary of what was accomplished", true),
                ToolParam::new("success", ParamType::Boolean, "Whether the task completed successfully (default: true)", false),
            ],
            false,
        ));

        // 16. kb_write
        self.register(ToolSpec::new(
            "kb_write",
            "Write or create a note in the knowledge base vault. Overwrites if the file already exists. Use for new notes, meeting summaries, calendar entries, and any content you want to persist.",
            "在知識庫寫入或創建筆記。若文件已存在則覆蓋。適用於新筆記、會議記錄、行程等需要持久化的內容。",
            vec![
                ToolParam::new("path", ParamType::String, "File path relative to vault root, e.g. 'notes/meeting-2026-04-07.md'", true),
                ToolParam::new("content", ParamType::String, "Markdown content to write (do not include frontmatter — it is added automatically)", true),
                ToolParam::new("type", ParamType::String, "Note type: note | todo | calendar | journal | research (default: note)", false),
                ToolParam::new("date", ParamType::String, "Date in YYYY-MM-DD format (default: today)", false),
                ToolParam::new("tags", ParamType::String, "Comma-separated tags, e.g. 'work,meeting,q2'", false),
            ],
            false,
        ));

        // 17. kb_read
        self.register(ToolSpec::new(
            "kb_read",
            "Read the full content of a note from the knowledge base vault by its path.",
            "根據路徑讀取知識庫中筆記的完整內容。",
            vec![
                ToolParam::new("path", ParamType::String, "File path relative to vault root, e.g. 'todos/2026-04-07.md'", true),
            ],
            false,
        ));

        // 18. kb_search
        self.register(ToolSpec::new(
            "kb_search",
            "Full-text search across all notes in the knowledge base vault. Use this to find past notes, todos, or any previously saved content.",
            "在知識庫所有筆記中進行全文搜索。用於查找過去的筆記、待辦事項或任何已保存的內容。",
            vec![
                ToolParam::new("query", ParamType::String, "Search query. Case-insensitive, searches all .md files in the vault.", true),
            ],
            false,
        ));

        // 19. kb_append
        self.register(ToolSpec::new(
            "kb_append",
            "Append content to an existing note without overwriting it. Use for adding items to a list, logging new entries, or extending notes.",
            "在已有筆記末尾追加內容，不覆蓋原有內容。適用於向清單添加條目、記錄新事項或擴展筆記。",
            vec![
                ToolParam::new("path", ParamType::String, "File path relative to vault root, e.g. 'todos/shopping.md'", true),
                ToolParam::new("content", ParamType::String, "Markdown content to append at the end of the file", true),
            ],
            false,
        ));

        // 20. kb_add_todo
        self.register(ToolSpec::new(
            "kb_add_todo",
            "Add a todo item to today's todo list. Creates the file automatically if it does not exist.",
            "新增待辦事項到今日的 todo 清單。若文件不存在則自動創建。",
            vec![
                ToolParam::new("text", ParamType::String, "The todo item text", true),
                ToolParam::new("due", ParamType::String, "Optional due date in YYYY-MM-DD format", false),
                ToolParam::new("priority", ParamType::String, "Optional priority: high | medium | low", false),
            ],
            false,
        ));
    }

    /// Registers the 8 mobile-only tools.
    fn register_mobile_tools(&mut self) {
        // 21. tap
        self.register(ToolSpec::new(
            "tap",
            "Tap on the screen at the specified coordinates. Use get_screen_info first to determine valid coordinate ranges.",
            "在指定座標點擊屏幕。先使用 get_screen_info 確定有效的座標範圍。",
            vec![
                ToolParam::new("x", ParamType::Integer, "X coordinate to tap", true),
                ToolParam::new("y", ParamType::Integer, "Y coordinate to tap", true),
            ],
            true,
        ));

        // 22. tap_node
        self.register(ToolSpec::new(
            "tap_node",
            "Tap on a specific UI node found by text, resource ID, or description. More reliable than coordinate-based tapping.",
            "點擊通過文本、資源 ID 或描述找到的特定 UI 節點。比基於座標的點擊更可靠。",
            vec![
                ToolParam::new("text", ParamType::String, "Text to search for in the UI node", false),
                ToolParam::new("resource_id", ParamType::String, "Resource ID to match", false),
                ToolParam::new("description", ParamType::String, "Content description to match", false),
                ToolParam::new("index", ParamType::Integer, "Index of the matching node when multiple matches exist (0-based, default: 0)", false),
            ],
            true,
        ));

        // 23. long_press
        self.register(ToolSpec::new(
            "long_press",
            "Long press on the screen at the specified coordinates or on a UI node. Useful for context menus and drag operations.",
            "在指定座標或 UI 節點長按。適用於上下文菜單和拖動操作。",
            vec![
                ToolParam::new("x", ParamType::Integer, "X coordinate to long press", true),
                ToolParam::new("y", ParamType::Integer, "Y coordinate to long press", true),
                ToolParam::new("duration_ms", ParamType::Integer, "Duration of the long press in milliseconds (default: 500)", false),
            ],
            true,
        ));

        // 24. swipe
        self.register(ToolSpec::new(
            "swipe",
            "Swipe from one coordinate to another. Use for scrolling, page navigation, or gesture-based interactions.",
            "從一個座標滑動到另一個座標。用於滾動、頁面導航或基於手勢的交互。",
            vec![
                ToolParam::new("start_x", ParamType::Integer, "Starting X coordinate", true),
                ToolParam::new("start_y", ParamType::Integer, "Starting Y coordinate", true),
                ToolParam::new("end_x", ParamType::Integer, "Ending X coordinate", true),
                ToolParam::new("end_y", ParamType::Integer, "Ending Y coordinate", true),
                ToolParam::new("duration_ms", ParamType::Integer, "Duration of the swipe in milliseconds (default: 300)", false),
            ],
            true,
        ));

        // 25. scroll_to_find
        self.register(ToolSpec::new(
            "scroll_to_find",
            "Scroll through a scrollable container to find a specific text or element. Stops when found or after max scrolls.",
            "在可滾動容器中滾動以查找特定文本或元素。找到時或達到最大滾動次數後停止。",
            vec![
                ToolParam::new("text", ParamType::String, "Text to search for while scrolling", true),
                ToolParam::new("direction", ParamType::String, "Scroll direction: 'up' or 'down' (default: 'down')", false),
                ToolParam::new("max_scrolls", ParamType::Integer, "Maximum number of scroll iterations (default: 10)", false),
            ],
            true,
        ));

        // 26. find_and_tap
        self.register(ToolSpec::new(
            "find_and_tap",
            "Find a UI element by text and tap it. Combines find_node_info and tap into a single action. Scrolls to find if needed.",
            "通過文本查找 UI 元素並點擊。將 find_node_info 和 tap 結合為一個操作。必要時會滾動查找。",
            vec![
                ToolParam::new("text", ParamType::String, "Text of the element to find and tap", true),
                ToolParam::new("scroll", ParamType::Boolean, "Whether to scroll to find the element if not visible (default: true)", false),
            ],
            true,
        ));

        // 27. send_message
        self.register(ToolSpec::new(
            "send_message",
            "Send a text message to a contact via a specified messaging app. Handles opening the app, finding the contact, and sending.",
            "通過指定的消息應用向聯繫人發送文本消息。處理打開應用、查找聯繫人和發送。",
            vec![
                ToolParam::new("contact", ParamType::String, "Contact name or phone number", true),
                ToolParam::new("message", ParamType::String, "Message text to send", true),
                ToolParam::new("app", ParamType::String, "Messaging app to use (e.g. 'whatsapp', 'telegram', 'line')", true),
            ],
            true,
        ));

        // 28. auto_reply
        self.register(ToolSpec::new(
            "auto_reply",
            "Automatically reply to an incoming message notification. Reads the notification content and generates a contextual reply.",
            "自動回覆收到的消息通知。讀取通知內容並生成上下文相關的回覆。",
            vec![
                ToolParam::new("notification_id", ParamType::String, "ID of the notification to reply to", true),
                ToolParam::new("reply_text", ParamType::String, "Reply text to send (if omitted, generates an automatic reply)", false),
            ],
            true,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_param_schema_property_string() {
        let param = ToolParam::new("name", ParamType::String, "A name", true);
        let schema = param.to_schema_property();
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["description"], "A name");
    }

    #[test]
    fn test_tool_param_schema_property_integer() {
        let param = ToolParam::new("x", ParamType::Integer, "X coordinate", true);
        let schema = param.to_schema_property();
        assert_eq!(schema["type"], "integer");
        assert_eq!(schema["description"], "X coordinate");
    }

    #[test]
    fn test_tool_param_schema_property_boolean() {
        let param = ToolParam::new("scroll", ParamType::Boolean, "Whether to scroll", false);
        let schema = param.to_schema_property();
        assert_eq!(schema["type"], "boolean");
    }

    #[test]
    fn test_tool_param_schema_property_number() {
        let param = ToolParam::new("ratio", ParamType::Number, "Scale factor", false);
        let schema = param.to_schema_property();
        assert_eq!(schema["type"], "number");
    }

    #[test]
    fn test_tool_spec_json_schema_with_required_fields() {
        let spec = ToolSpec::new(
            "tap",
            "Tap the screen",
            "點擊屏幕",
            vec![
                ToolParam::new("x", ParamType::Integer, "X coordinate", true),
                ToolParam::new("y", ParamType::Integer, "Y coordinate", true),
            ],
            true,
        );

        let schema = spec.parameters_json_schema();

        // Verify top-level structure
        assert_eq!(schema["type"], "object");

        // Verify properties exist
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("x"));
        assert!(props.contains_key("y"));
        assert_eq!(props["x"]["type"], "integer");
        assert_eq!(props["y"]["type"], "integer");

        // Verify required fields
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&json!("x")));
        assert!(required.contains(&json!("y")));
    }

    #[test]
    fn test_tool_spec_json_schema_with_optional_fields() {
        let spec = ToolSpec::new(
            "test_tool",
            "A test",
            "測試",
            vec![
                ToolParam::new("required_param", ParamType::String, "Required", true),
                ToolParam::new("optional_param", ParamType::Integer, "Optional", false),
            ],
            false,
        );

        let schema = spec.parameters_json_schema();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert!(required.contains(&json!("required_param")));
        assert!(!required.contains(&json!("optional_param")));
    }

    #[test]
    fn test_tool_spec_json_schema_no_params() {
        let spec = ToolSpec::new(
            "get_screen_info",
            "Get screen info",
            "獲取屏幕信息",
            vec![],
            false,
        );

        let schema = spec.parameters_json_schema();
        assert_eq!(schema["type"], "object");
        let props = schema["properties"].as_object().unwrap();
        assert!(props.is_empty());
        let required = schema["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_default_registry_tool_count() {
        let registry = ToolRegistry::default();
        // 20 common + 8 mobile = 28 total
        assert_eq!(registry.len(), 28);
        assert_eq!(registry.common_tools().len(), 20);
        assert_eq!(registry.mobile_tools().len(), 8);
    }

    #[test]
    fn test_registry_get_existing_tool() {
        let registry = ToolRegistry::default();
        let tap = registry.get("tap").expect("tap should exist");
        assert_eq!(tap.name, "tap");
        assert!(tap.is_mobile);
    }

    #[test]
    fn test_registry_get_nonexistent_tool() {
        let registry = ToolRegistry::default();
        assert!(registry.get("nonexistent_tool").is_none());
    }

    #[test]
    fn test_tap_parameter_schema() {
        let registry = ToolRegistry::default();
        let tap = registry.get("tap").expect("tap should exist");
        let schema = tap.parameters_json_schema();

        // tap should have required x and y as integers
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&json!("x")));
        assert!(required.contains(&json!("y")));

        let props = schema["properties"].as_object().unwrap();
        assert_eq!(props["x"]["type"], "integer");
        assert_eq!(props["y"]["type"], "integer");
    }

    #[test]
    fn test_open_app_parameter_schema() {
        let registry = ToolRegistry::default();
        let open_app = registry.get("open_app").expect("open_app should exist");
        assert!(!open_app.is_mobile);

        let schema = open_app.parameters_json_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("app_name"));
        assert_eq!(props["app_name"]["type"], "string");

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("app_name")));
    }

    #[test]
    fn test_all_tool_names_unique() {
        let registry = ToolRegistry::default();
        let all_names: Vec<&str> = registry.all_specs().iter().map(|s| s.name.as_str()).collect();
        let mut unique_names = all_names.clone();
        unique_names.sort();
        unique_names.dedup();
        assert_eq!(all_names.len(), unique_names.len(), "Duplicate tool names detected");
    }

    #[test]
    fn test_all_expected_common_tools_present() {
        let registry = ToolRegistry::default();
        let common_names = [
            "get_screen_info", "find_node_info", "input_text", "system_key",
            "open_app", "get_installed_apps", "take_screenshot", "wait",
            "repeat_actions", "clipboard", "send_file", "get_device_info",
            "get_notifications", "make_call", "finish",
            "kb_write", "kb_read", "kb_search", "kb_append", "kb_add_todo",
        ];
        for name in &common_names {
            let tool = registry.get(name).unwrap_or_else(|| panic!("Common tool '{}' not found", name));
            assert!(!tool.is_mobile, "Common tool '{}' should not be mobile", name);
        }
    }

    #[test]
    fn test_all_expected_mobile_tools_present() {
        let registry = ToolRegistry::default();
        let mobile_names = [
            "tap", "tap_node", "long_press", "swipe",
            "scroll_to_find", "find_and_tap", "send_message", "auto_reply",
        ];
        for name in &mobile_names {
            let tool = registry.get(name).unwrap_or_else(|| panic!("Mobile tool '{}' not found", name));
            assert!(tool.is_mobile, "Mobile tool '{}' should be mobile", name);
        }
    }

    #[test]
    fn test_description_language_toggle() {
        let registry = ToolRegistry::default();
        let tap = registry.get("tap").unwrap();
        assert_eq!(tap.description(false), "Tap on the screen at the specified coordinates. Use get_screen_info first to determine valid coordinate ranges.");
        assert!(tap.description(true).contains("點擊屏幕"));
    }

    #[test]
    fn test_kb_write_schema_matches_kotlin() {
        let registry = ToolRegistry::default();
        let kb_write = registry.get("kb_write").expect("kb_write should exist");
        let schema = kb_write.parameters_json_schema();

        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("content"));
        assert!(props.contains_key("type"));
        assert!(props.contains_key("date"));
        assert!(props.contains_key("tags"));

        // path and content are required; type, date, tags are optional
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("content")));
        assert_eq!(required.len(), 2);
    }

    #[test]
    fn test_swipe_parameter_schema() {
        let registry = ToolRegistry::default();
        let swipe = registry.get("swipe").expect("swipe should exist");
        let schema = swipe.parameters_json_schema();

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("start_x")));
        assert!(required.contains(&json!("start_y")));
        assert!(required.contains(&json!("end_x")));
        assert!(required.contains(&json!("end_y")));
        assert_eq!(required.len(), 4); // duration_ms is optional

        let props = schema["properties"].as_object().unwrap();
        assert_eq!(props["start_x"]["type"], "integer");
        assert_eq!(props["start_y"]["type"], "integer");
        assert_eq!(props["end_x"]["type"], "integer");
        assert_eq!(props["end_y"]["type"], "integer");
        assert_eq!(props["duration_ms"]["type"], "integer");
    }

    #[test]
    fn test_empty_registry() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.get("tap").is_none());
    }

    #[test]
    fn test_register_overwrites_existing() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolSpec::new(
            "test", "desc1", "desc_cn1", vec![], false,
        ));
        registry.register(ToolSpec::new(
            "test", "desc2", "desc_cn2", vec![], false,
        ));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("test").unwrap().description_en, "desc2");
    }
}
