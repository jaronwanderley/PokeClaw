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
