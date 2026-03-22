use crate::GlobalShortcutState;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

#[derive(serde::Deserialize)]
pub struct GlobalShortcutBinding {
    pub id: String,
    pub key: String,
}

fn default_key_for_id(id: &str) -> Option<&'static str> {
    match id {
        "show_app" => Some("CommandOrControl+Shift+J"),
        "format_clipboard" => Some("CommandOrControl+Shift+V"),
        _ => None,
    }
}

fn unregister_key(app: &AppHandle, key: &str) {
    let _ = app.global_shortcut().unregister(key);
}

fn register_global_shortcut(
    app: &AppHandle,
    id: &str,
    shortcut: Shortcut,
) -> Result<(), String> {
    match id {
        "show_app" => {
            let app_handle = app.clone();
            app.global_shortcut()
                .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = show_main_window(handle).await;
                    });
                })
                .map_err(|e| format!("Failed to register shortcut: {}", e))?;
        }
        "format_clipboard" => {
            let app_handle = app.clone();
            app.global_shortcut()
                .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = format_clipboard_and_show(handle).await;
                    });
                })
                .map_err(|e| format!("Failed to register shortcut: {}", e))?;
        }
        _ => return Err("Unknown shortcut id".to_string()),
    }

    Ok(())
}

#[tauri::command]
pub async fn update_shortcut(
    app: AppHandle,
    state: State<'_, GlobalShortcutState>,
    id: String,
    key: String,
) -> Result<(), String> {
    if default_key_for_id(&id).is_none() {
        return Err("Unknown shortcut id".to_string());
    }

    // Unregister old shortcut (from state; fallback to default for compatibility)
    let old_key = {
        let map = state.0.lock().unwrap();
        map.get(&id).cloned()
    };
    if let Some(old_key) = old_key.as_deref() {
        unregister_key(&app, old_key);
    }
    if let Some(default_key) = default_key_for_id(&id) {
        unregister_key(&app, default_key);
    }

    // Empty key means "disable this global shortcut"
    if key.trim().is_empty() {
        let mut map = state.0.lock().unwrap();
        map.remove(&id);
        return Ok(());
    }

    // Parse shortcut string
    let shortcut: Shortcut = key
        .parse()
        .map_err(|e| format!("Invalid shortcut format: {:?}", e))?;

    // Register new shortcut
    register_global_shortcut(&app, &id, shortcut)?;

    // Update state
    let mut map = state.0.lock().unwrap();
    map.insert(id, key);
    Ok(())
}

#[tauri::command]
pub async fn sync_global_shortcuts(
    app: AppHandle,
    state: State<'_, GlobalShortcutState>,
    enabled: bool,
    bindings: Vec<GlobalShortcutBinding>,
) -> Result<(), String> {
    // Validate ids up-front to fail fast.
    for b in &bindings {
        if default_key_for_id(&b.id).is_none() {
            return Err(format!("Unknown shortcut id: {}", b.id));
        }
    }

    // Collect keys we might need to unregister (state + defaults + provided bindings)
    let mut keys_to_unregister: Vec<String> = Vec::new();
    {
        let map = state.0.lock().unwrap();
        keys_to_unregister.extend(map.values().cloned());
    }
    for id in ["show_app", "format_clipboard"] {
        if let Some(default_key) = default_key_for_id(id) {
            keys_to_unregister.push(default_key.to_string());
        }
    }
    for b in &bindings {
        if !b.key.trim().is_empty() {
            keys_to_unregister.push(b.key.clone());
        }
    }
    keys_to_unregister.sort();
    keys_to_unregister.dedup();

    for key in &keys_to_unregister {
        unregister_key(&app, key);
    }

    // Reset state to avoid stale entries
    {
        let mut map = state.0.lock().unwrap();
        map.clear();
    }

    if !enabled {
        return Ok(());
    }

    // Parse all shortcuts before registering (avoid partial registration when invalid)
    let mut parsed: Vec<(String, String, Shortcut)> = Vec::new();
    for b in bindings {
        let key = b.key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let shortcut: Shortcut = key
            .parse()
            .map_err(|e| format!("Invalid shortcut format: {:?}", e))?;
        parsed.push((b.id, key, shortcut));
    }

    for (id, key, shortcut) in parsed {
        register_global_shortcut(&app, &id, shortcut)?;
        let mut map = state.0.lock().unwrap();
        map.insert(id, key);
    }

    Ok(())
}

#[tauri::command]
pub async fn show_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        ensure_window_in_front(&window)?;
        Ok(())
    } else {
        Err("Main window not found".to_string())
    }
}

#[tauri::command]
pub async fn format_clipboard_and_show(app: AppHandle) -> Result<(), String> {
    // Get clipboard content
    let clipboard_text = app
        .clipboard()
        .read_text()
        .map_err(|e| format!("Failed to read clipboard: {}", e))?;

    if clipboard_text.is_empty() {
        return Err("Clipboard is empty".to_string());
    }

    // Show window first
    let window = app
        .get_webview_window("main")
        .ok_or("Main window not found".to_string())?;

    ensure_window_in_front(&window)?;

    // Try to parse and format JSON
    match serde_json::from_str::<serde_json::Value>(&clipboard_text) {
        Ok(parsed) => {
            // Valid JSON - format it
            let formatted = serde_json::to_string_pretty(&parsed)
                .map_err(|e| format!("Failed to format JSON: {}", e))?;

            window
                .emit("clipboard-formatted", formatted)
                .map_err(|e| e.to_string())?;
        }
        Err(_) => {
            // Invalid JSON - paste as is, let user see and fix it
            window
                .emit("clipboard-pasted-raw", clipboard_text)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

fn ensure_window_in_front(window: &WebviewWindow) -> Result<(), String> {
    let mut elevated = false;

    if window.is_minimized().map_err(|e| e.to_string())? {
        window.unminimize().map_err(|e| e.to_string())?;
        elevated = true;
    }

    if !window.is_visible().map_err(|e| e.to_string())? {
        window.show().map_err(|e| e.to_string())?;
        elevated = true;
    }

    if !window.is_focused().map_err(|e| e.to_string())? {
        window.set_focus().map_err(|e| e.to_string())?;
        elevated = true;
    }

    if elevated {
        window.set_always_on_top(true).map_err(|e| e.to_string())?;
        let window_clone = window.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let _ = window_clone.set_always_on_top(false);
        });
    }

    Ok(())
}
