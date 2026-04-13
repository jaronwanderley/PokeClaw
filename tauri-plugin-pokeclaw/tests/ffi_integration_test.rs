// tests/ffi_integration_test.rs — Integration tests for LiteRT-LM FFI and model management.
//
// Tests are designed to gracefully skip when native components are unavailable:
// - Library loading tests skip if litertlm_bridge.dll/.so/.dylib is not found.
// - Engine creation tests skip if no .litertlm model file is available.
// - Model manager tests run unconditionally (pure filesystem operations).

use std::path::PathBuf;

/// Candidate paths to search for the LiteRT-LM bridge library.
fn find_bridge_library() -> Option<PathBuf> {
    let candidates = [
        // Next to executable
        std::env::current_exe()
            .ok()?
            .parent()?
            .join("litertlm_bridge.dll"),
        std::env::current_exe()
            .ok()?
            .parent()?
            .join("litertlm_bridge.so"),
        std::env::current_exe()
            .ok()?
            .parent()?
            .join("litertlm_bridge.dylib"),
        // Build output
        PathBuf::from("target/release/litertlm_bridge.dll"),
        PathBuf::from("target/release/litertlm_bridge.so"),
        PathBuf::from("target/debug/litertlm_bridge.dll"),
        PathBuf::from("target/debug/litertlm_bridge.so"),
        // CMake build output
        PathBuf::from(
            "tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.dll",
        ),
        PathBuf::from(
            "tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.so",
        ),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            eprintln!("[ffi_integration_test] Found bridge library: {}", candidate.display());
            return Some(candidate.clone());
        }
    }
    None
}

/// Candidate paths to search for a .litertlm model file.
fn find_model_file() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("models/gemma-4-E2B-it.litertlm"),
        PathBuf::from("models/gemma-4-E4B-it.litertlm"),
        PathBuf::from("test-model.litertlm"),
        // Check alongside executable
        std::env::current_exe()
            .ok()?
            .parent()?
            .join("models/gemma-4-E2B-it.litertlm"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            eprintln!("[ffi_integration_test] Found model file: {}", candidate.display());
            return Some(candidate.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Test: Library loading
// ---------------------------------------------------------------------------

#[test]
fn test_library_loading() {
    let lib_path = match find_bridge_library() {
        Some(p) => p,
        None => {
            eprintln!(
                "[SKIP] test_library_loading: litertlm_bridge library not found. \
                 Build the C++ shim to enable this test."
            );
            return;
        }
    };

    eprintln!("[test_library_loading] Attempting to load: {}", lib_path.display());

    // Try to load the library via libloading (same as LitertEngine::new)
    let result = unsafe { libloading::Library::new(&lib_path) };

    match result {
        Ok(_lib) => {
            eprintln!("[PASS] test_library_loading: library loaded successfully");
        }
        Err(e) => {
            panic!(
                "[FAIL] test_library_loading: failed to load library '{}': {}",
                lib_path.display(),
                e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test: Engine creation
// ---------------------------------------------------------------------------

#[test]
fn test_engine_creation() {
    let lib_path = match find_bridge_library() {
        Some(p) => p,
        None => {
            eprintln!(
                "[SKIP] test_engine_creation: litertlm_bridge library not found."
            );
            return;
        }
    };

    let model_path = match find_model_file() {
        Some(p) => p,
        None => {
            eprintln!(
                "[SKIP] test_engine_creation: no .litertlm model file found. \
                 Download a model to enable this test."
            );
            return;
        }
    };

    eprintln!(
        "[test_engine_creation] lib={}, model={}",
        lib_path.display(),
        model_path.display()
    );

    // Load library and create engine via FFI (using the plugin's FFI module)
    // We import the types from the plugin crate
    use tauri_plugin_pokeclaw::_ffi_test_exports::*;

    let mut engine = match LitertEngineShim::new(&lib_path) {
        Ok(e) => e,
        Err(e) => {
            panic!(
                "[FAIL] test_engine_creation: failed to load library: {}",
                e
            );
        }
    };

    match engine.load_model(&model_path, "cpu") {
        Ok(()) => {
            eprintln!(
                "[PASS] test_engine_creation: engine created and model loaded successfully"
            );
        }
        Err(e) => {
            panic!(
                "[FAIL] test_engine_creation: model load failed for '{}': {}",
                model_path.display(),
                e
            );
        }
    }

    // Verify model_path() is set
    assert_eq!(
        engine.model_path(),
        Some(model_path.to_string_lossy().to_string().as_str()),
        "model_path should match the loaded model"
    );
    assert_eq!(engine.backend(), Some("cpu"));

    // Drop the engine (tests cleanup)
    drop(engine);
    eprintln!("[PASS] test_engine_creation: engine dropped cleanly");
}

// ---------------------------------------------------------------------------
// Test: Session lifecycle (create conversation, send message)
// ---------------------------------------------------------------------------

#[test]
fn test_session_lifecycle() {
    let lib_path = match find_bridge_library() {
        Some(p) => p,
        None => {
            eprintln!(
                "[SKIP] test_session_lifecycle: litertlm_bridge library not found."
            );
            return;
        }
    };

    let model_path = match find_model_file() {
        Some(p) => p,
        None => {
            eprintln!(
                "[SKIP] test_session_lifecycle: no .litertlm model file found."
            );
            return;
        }
    };

    eprintln!(
        "[test_session_lifecycle] lib={}, model={}",
        lib_path.display(),
        model_path.display()
    );

    use tauri_plugin_pokeclaw::_ffi_test_exports::*;

    let mut engine = LitertEngineShim::new(&lib_path)
        .expect("Library should load (already verified in test_library_loading)");

    engine
        .load_model(&model_path, "cpu")
        .expect("Model should load (already verified in test_engine_creation)");

    // Create a conversation
    let session = engine
        .create_conversation()
        .expect("Conversation creation should succeed");

    let session_id = session.session_id().to_string();
    eprintln!(
        "[test_session_lifecycle] conversation created — session_id={}",
        session_id
    );
    assert!(
        session_id.starts_with("litert-"),
        "Session ID should start with 'litert-'"
    );
    assert_eq!(session.backend(), "cpu");

    // Send a simple message
    let response = session
        .send_message("Hello, this is an integration test.")
        .expect("send_message should succeed");

    eprintln!(
        "[test_session_lifecycle] send_message response ({} chars): '{}'",
        response.len(),
        if response.len() > 100 {
            &response[..100]
        } else {
            &response
        }
    );
    assert!(
        !response.is_empty(),
        "Response should not be empty for a valid prompt"
    );

    // Drop session (tests conversation cleanup)
    drop(session);

    // Create another conversation on the same engine
    let session2 = engine
        .create_conversation()
        .expect("Second conversation creation should succeed on same engine");

    eprintln!(
        "[test_session_lifecycle] second conversation — session_id={}",
        session2.session_id()
    );
    assert_ne!(
        session2.session_id(),
        session_id,
        "Second session should have a different ID"
    );

    drop(session2);
    drop(engine);

    eprintln!("[PASS] test_session_lifecycle: full lifecycle completed cleanly");
}

// ---------------------------------------------------------------------------
// Test: Model manager — list_models
// ---------------------------------------------------------------------------

#[test]
fn test_model_manager_list_models() {
    // list_models is a pure filesystem operation — always runs
    use tauri_plugin_pokeclaw::_ffi_test_exports::list_models;

    let models = list_models();
    assert!(!models.is_empty(), "list_models should return at least one model");

    for model in &models {
        assert!(!model.id.is_empty(), "Model ID should not be empty");
        assert!(
            model.file_name.ends_with(".litertlm"),
            "Model file_name should end with .litertlm, got: {}",
            model.file_name
        );
        assert!(!model.url.is_empty(), "Model URL should not be empty");
        assert!(model.size_bytes > 0, "Model size_bytes should be > 0");
        assert!(model.min_ram_gb > 0, "Model min_ram_gb should be > 0");

        eprintln!(
            "[test_model_manager_list_models] model_id={}, is_downloaded={}, local_path={:?}",
            model.id,
            model.is_downloaded,
            model.local_path
        );
    }

    eprintln!("[PASS] test_model_manager_list_models: catalog structure verified");
}

// ---------------------------------------------------------------------------
// Test: Model manager — models_dir
// ---------------------------------------------------------------------------

#[test]
fn test_model_manager_models_dir() {
    use tauri_plugin_pokeclaw::_ffi_test_exports::models_dir;

    let dir = models_dir();
    assert!(
        !dir.as_os_str().is_empty(),
        "models_dir should not return an empty path"
    );
    assert!(
        dir.to_string_lossy().ends_with("models"),
        "Default models_dir should end with 'models', got: {}",
        dir.display()
    );

    eprintln!(
        "[PASS] test_model_manager_models_dir: dir='{}'",
        dir.display()
    );
}

// ---------------------------------------------------------------------------
// Test: Error display — LitertError formatting
// ---------------------------------------------------------------------------

#[test]
fn test_litert_error_display_formatting() {
    use tauri_plugin_pokeclaw::_ffi_test_exports::LitertError;

    let err = LitertError::LibraryNotFound {
        path: "/test/path.so".into(),
        source: libloading::Error::LoadLibraryExWUnknown,
    };
    let msg = format!("{}", err);
    assert!(msg.contains("/test/path.so"), "Error should contain the path");
    assert!(
        msg.contains("Build the C++ shim"),
        "Error should contain build instructions"
    );

    let err2 = LitertError::EngineCreateFailed {
        model_path: "/models/test.litertlm".into(),
        detail: "model not found".into(),
    };
    let msg2 = format!("{}", err2);
    assert!(msg2.contains("/models/test.litertlm"));
    assert!(msg2.contains("model not found"));

    let err3 = LitertError::InvalidModelPath("".into());
    let msg3 = format!("{}", err3);
    assert!(msg3.contains("Invalid model path"));

    eprintln!("[PASS] test_litert_error_display_formatting: all error formats verified");
}
