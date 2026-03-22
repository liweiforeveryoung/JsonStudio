mod commands;

use commands::clipboard::copy_image_to_clipboard;
use commands::codegen::{code_to_json, json_to_code};
use commands::convert::{
    csv_to_json, json_to_csv, json_to_toml, json_to_xml, json_to_yaml, toml_to_json, xml_to_json,
    yaml_to_json,
};
use commands::export_image::export_json_image;
use commands::file::{
    get_file_name, is_json_file, open_file_dialog, read_file, save_file, save_file_dialog,
};
use commands::file_watcher::{unwatch_all_files, unwatch_file, watch_file, FileWatcherState};
use commands::json::{
    extract_json_fragments, json_escape, json_format, json_minify, json_stats, json_unescape,
    json_validate,
};
use commands::shortcuts::{format_clipboard_and_show, show_main_window, update_shortcut};
use commands::window::{open_devtools, set_window_theme};
use std::sync::Mutex;
use tauri::Emitter;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

static PENDING_FILES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FRONTEND_READY: Mutex<bool> = Mutex::new(false);

#[tauri::command]
fn get_pending_files() -> Vec<String> {
    *FRONTEND_READY.lock().unwrap() = true;
    PENDING_FILES.lock().unwrap().drain(..).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(FileWatcherState::new())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Register global shortcut: show app
            let show_app_handle = app_handle.clone();
            app.global_shortcut()
                .on_shortcut(
                    "CommandOrControl+Shift+J",
                    move |_app, _shortcut, _event| {
                        let handle = show_app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = show_main_window(handle).await;
                        });
                    },
                )
                .map_err(|e| format!("Failed to register show app shortcut: {}", e))?;

            // Register global shortcut: format clipboard
            let format_handle = app_handle.clone();
            app.global_shortcut()
                .on_shortcut(
                    "CommandOrControl+Shift+V",
                    move |_app, _shortcut, _event| {
                        let handle = format_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = format_clipboard_and_show(handle).await;
                        });
                    },
                )
                .map_err(|e| format!("Failed to register format clipboard shortcut: {}", e))?;

            #[cfg(not(target_os = "macos"))]
            {
                use std::path::Path;
                let paths: Vec<String> = std::env::args()
                    .skip(1)
                    .filter(|arg| !arg.starts_with('-'))
                    .filter(|arg| {
                        let p = Path::new(arg);
                        p.exists()
                            && p.extension()
                                .and_then(|e| e.to_str())
                                .map(|e| e.eq_ignore_ascii_case("json"))
                                .unwrap_or(false)
                    })
                    .collect();

                if !paths.is_empty() {
                    PENDING_FILES.lock().unwrap().extend(paths);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            json_format,
            json_minify,
            json_validate,
            json_stats,
            json_escape,
            json_unescape,
            extract_json_fragments,
            set_window_theme,
            open_devtools,
            show_main_window,
            format_clipboard_and_show,
            update_shortcut,
            open_file_dialog,
            save_file,
            save_file_dialog,
            read_file,
            is_json_file,
            get_file_name,
            watch_file,
            unwatch_file,
            unwatch_all_files,
            json_to_yaml,
            json_to_toml,
            json_to_xml,
            json_to_csv,
            yaml_to_json,
            toml_to_json,
            xml_to_json,
            csv_to_json,
            json_to_code,
            code_to_json,
            copy_image_to_clipboard,
            export_json_image,
            get_pending_files
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = event {
            let paths: Vec<String> = urls
                .iter()
                .filter_map(|url| {
                    if url.scheme() == "file" {
                        url.to_file_path()
                            .ok()
                            .map(|p| p.to_string_lossy().into_owned())
                    } else {
                        None
                    }
                })
                .collect();

            if !paths.is_empty() {
                let ready = *FRONTEND_READY.lock().unwrap();
                if ready {
                    let _ = app_handle.emit("open-file", paths);
                } else {
                    PENDING_FILES.lock().unwrap().extend(paths);
                }
            }
        }
    });
}
