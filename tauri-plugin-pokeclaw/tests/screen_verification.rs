use tauri_plugin_pokeclaw::desktop::screen;
use std::fs;

#[test]
fn test_take_screenshot() {
    let path = "test_screenshot.png";
    
    // Clean up if it exists
    if fs::metadata(path).is_ok() {
        fs::remove_file(path).unwrap();
    }

    let result = screen::do_take_screenshot(Some(path));
    
    assert!(result.success, "Screenshot should be successful: {:?}", result.error);
    assert!(fs::metadata(path).is_ok(), "Screenshot file should exist");
    
    // Check if it's a valid PNG (simple check for header)
    let bytes = fs::read(path).unwrap();
    assert!(bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]), "File should have PNG header");

    // Clean up
    fs::remove_file(path).unwrap();
}

#[test]
fn test_get_screen_info() {
    let result = screen::do_get_screen_info();
    assert!(result.success, "get_screen_info should be successful: {:?}", result.error);
    let data = result.data.unwrap();
    let tree = data.get("tree").unwrap().as_str().unwrap();
    println!("Screen Tree:\n{}", tree);
    // Tree should not be empty if windows are open
    assert!(!tree.is_empty(), "Screen tree should not be empty");
}

#[test]
fn test_find_node_info() {
    // Search for something that likely exists in the tree we just got
    let result_all = screen::do_get_screen_info();
    let tree = result_all.data.unwrap().get("tree").unwrap().as_str().unwrap().to_string();
    
    if let Some(first_line) = tree.lines().next() {
        // Extract title from "[n1] \"Title\" (App) at ..."
        if let Some(start) = first_line.find('"') {
            if let Some(end) = first_line[start+1..].find('"') {
                let title = &first_line[start+1..start+1+end];
                if !title.is_empty() {
                    let result = screen::do_find_node_info(title.to_string());
                    assert!(result.success);
                    let nodes = result.data.unwrap().get("nodes").unwrap().as_array().unwrap().to_vec();
                    assert!(!nodes.is_empty(), "Should find at least one node for title '{}'", title);
                }
            }
        }
    }
}
