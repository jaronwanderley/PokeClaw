// desktop/kb.rs — Real filesystem-based Knowledge Base tools for desktop (non-Android).

use std::path::{Path, PathBuf};
use std::fs;
use crate::ToolResult;
use serde_json::json;

// ---------------------------------------------------------------------------
// Knowledge Base directory
// ---------------------------------------------------------------------------

/// Returns the default Knowledge Base directory path.
///
/// Default: `{current_exe_parent}/kb/`.
/// Can be overridden via the `POKECLAW_KB_DIR` environment variable.
pub fn kb_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("POKECLAW_KB_DIR") {
        let path = PathBuf::from(custom);
        log::info!("kb_dir: using custom path from POKECLAW_KB_DIR='{}'", path.display());
        return path;
    }

    // Default: next to the executable
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let dir = exe_dir.join("kb");
    log::debug!("kb_dir: using default path='{}'", dir.display());
    dir
}

/// Helper to resolve a relative path within the KB directory and ensure it stays inside.
fn resolve_kb_path(relative_path: &str) -> Result<PathBuf, String> {
    let kb_root = kb_dir();
    
    // Check for directory traversal in relative_path
    let rel_path = Path::new(relative_path);
    if rel_path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))) {
        return Err("Security error: Invalid path components in Knowledge Base path".into());
    }

    // Create KB root if it doesn't exist
    if !kb_root.exists() {
        fs::create_dir_all(&kb_root).map_err(|e| format!("Failed to create KB directory: {}", e))?;
    }

    let joined = kb_root.join(relative_path);
    
    // Ensure the parent directory exists if we're writing a new file in a subdirectory
    if let Some(parent) = joined.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directory: {}", e))?;
        }
    }

    Ok(joined)
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

/// Writes content to a file in the Knowledge Base.
pub fn do_kb_write(path: &str, content: &str) -> ToolResult {
    log::info!("kb_write: path='{}', content_len={}", path, content.len());

    let full_path = match resolve_kb_path(path) {
        Ok(p) => p,
        Err(e) => return ToolResult { success: false, data: None, error: Some(e) },
    };

    match fs::write(&full_path, content) {
        Ok(_) => ToolResult {
            success: true,
            data: Some(json!({ "message": format!("Successfully written to '{}'", path) })),
            error: None,
        },
        Err(e) => ToolResult {
            success: false,
            data: None,
            error: Some(format!("Failed to write file '{}': {}", path, e)),
        },
    }
}

/// Reads content from a file in the Knowledge Base.
pub fn do_kb_read(path: &str) -> ToolResult {
    log::info!("kb_read: path='{}'", path);

    let full_path = match resolve_kb_path(path) {
        Ok(p) => p,
        Err(e) => return ToolResult { success: false, data: None, error: Some(e) },
    };

    if !full_path.exists() {
        return ToolResult {
            success: false,
            data: None,
            error: Some(format!("File '{}' does not exist", path)),
        };
    }

    match fs::read_to_string(&full_path) {
        Ok(content) => ToolResult {
            success: true,
            data: Some(json!({ "content": content })),
            error: None,
        },
        Err(e) => ToolResult {
            success: false,
            data: None,
            error: Some(format!("Failed to read file '{}': {}", path, e)),
        },
    }
}

/// Appends content to a file in the Knowledge Base.
pub fn do_kb_append(path: &str, content: &str) -> ToolResult {
    log::info!("kb_append: path='{}', content_len={}", path, content.len());

    let full_path = match resolve_kb_path(path) {
        Ok(p) => p,
        Err(e) => return ToolResult { success: false, data: None, error: Some(e) },
    };

    use std::io::Write;
    let mut file = match fs::OpenOptions::new().create(true).append(true).open(&full_path) {
        Ok(f) => f,
        Err(e) => return ToolResult {
            success: false,
            data: None,
            error: Some(format!("Failed to open file '{}' for append: {}", path, e)),
        },
    };

    if let Err(e) = file.write_all(content.as_bytes()) {
        return ToolResult {
            success: false,
            data: None,
            error: Some(format!("Failed to append to file '{}': {}", path, e)),
        };
    }

    ToolResult {
        success: true,
        data: Some(json!({ "message": format!("Successfully appended to '{}'", path) })),
        error: None,
    }
}

/// Appends a todo item to 'todo.md' in the Knowledge Base.
pub fn do_kb_add_todo(text: &str) -> ToolResult {
    let todo_line = format!("- [ ] {}\n", text);
    do_kb_append("todo.md", &todo_line)
}

/// Searches for a query in all files in the Knowledge Base directory.
pub fn do_kb_search(query: &str) -> ToolResult {
    log::info!("kb_search: query='{}'", query);

    let kb_root = kb_dir();
    if !kb_root.exists() {
        return ToolResult {
            success: true,
            data: Some(json!({ "results": [] })),
            error: None,
        };
    }

    let mut results = Vec::new();
    let query_lower = query.to_lowercase();

    fn visit_dirs(dir: &Path, query: &str, results: &mut Vec<serde_json::Value>) -> std::io::Result<()> {
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path, query, results)?;
                } else {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if content.to_lowercase().contains(query) {
                            let rel_path = path.strip_prefix(kb_dir()).unwrap_or(&path);
                            results.push(json!({
                                "path": rel_path.to_string_lossy(),
                                "preview": get_preview(&content, query)
                            }));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn get_preview(content: &str, query: &str) -> String {
        let content_lower = content.to_lowercase();
        if let Some(idx) = content_lower.find(query) {
            let start = idx.saturating_sub(30);
            let end = std::cmp::min(content.len(), idx + query.len() + 30);
            let preview = &content[start..end];
            format!("...{}...", preview.replace('\n', " "))
        } else {
            content.chars().take(60).collect::<String>().replace('\n', " ")
        }
    }

    if let Err(e) = visit_dirs(&kb_root, &query_lower, &mut results) {
        return ToolResult {
            success: false,
            data: None,
            error: Some(format!("Search failed: {}", e)),
        };
    }

    ToolResult {
        success: true,
        data: Some(json!({ "results": results })),
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_kb_write_read() {
        let dir = tempdir().unwrap();
        std::env::set_var("POKECLAW_KB_DIR", dir.path());

        let res = do_kb_write("test.md", "hello world");
        assert!(res.success);

        let res = do_kb_read("test.md");
        assert!(res.success);
        assert_eq!(res.data.unwrap()["content"], "hello world");
    }

    #[test]
    fn test_kb_append() {
        let dir = tempdir().unwrap();
        std::env::set_var("POKECLAW_KB_DIR", dir.path());

        do_kb_write("test.md", "hello");
        let res = do_kb_append("test.md", " world");
        assert!(res.success);

        let res = do_kb_read("test.md");
        assert_eq!(res.data.unwrap()["content"], "hello world");
    }

    #[test]
    fn test_kb_search() {
        let dir = tempdir().unwrap();
        std::env::set_var("POKECLAW_KB_DIR", dir.path());

        do_kb_write("note1.md", "Meeting about project X");
        do_kb_write("note2.md", "Shopping list: milk, eggs");
        do_kb_write("subdir/note3.md", "Project X deadline is tomorrow");

        let res = do_kb_search("project x");
        assert!(res.success);
        let results = res.data.unwrap()["results"].as_array().unwrap().clone();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_kb_path_security() {
        let dir = tempdir().unwrap();
        std::env::set_var("POKECLAW_KB_DIR", dir.path());

        let res = do_kb_write("../outside.txt", "evil");
        assert!(!res.success);
        assert!(res.error.unwrap().contains("Security error"));
    }
}
