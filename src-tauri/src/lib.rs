/// Launch the slicer executable with a model file. The exe path comes from
/// user settings, so this can't be expressed as a static shell-plugin scope.
#[tauri::command]
fn launch_slicer(exe: String, file: String) -> Result<(), String> {
    if !std::path::Path::new(&exe).is_file() {
        return Err(format!("slicer not found at {exe}"));
    }
    std::process::Command::new(&exe)
        .arg(&file)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![launch_slicer])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
