mod auth;
mod commands;
mod data;
mod model;
mod onshape;
mod selftest;
mod settings;
mod slicers;

use tauri::WebviewWindowBuilder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let client = onshape::http_client();
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(auth::Auth::new(client.clone()))
        .manage(slicers::Slicers::new())
        .manage(onshape::Onshape::new(client))
        .invoke_handler(tauri::generate_handler![
            commands::get_docs,
            commands::get_elements,
            commands::peek_parts,
            commands::get_parts,
            commands::get_thumb,
            commands::save_export,
            commands::print_export,
            commands::get_call_stats,
            auth::get_auth_settings,
            auth::save_auth_settings,
            slicers::list_slicers,
            slicers::rescan_slicers,
            slicers::pick_manual_slicer,
            slicers::select_slicer,
        ])
        .setup(|app| {
            if selftest::requested() {
                // Headless: no window, no webview.
                tauri::async_runtime::spawn(selftest::run_and_exit(app.handle().clone()));
            } else {
                // The window is declared in tauri.conf.json with create=false
                // so that the selftest can skip it.
                let config = app
                    .config()
                    .app
                    .windows
                    .first()
                    .ok_or("no window in tauri.conf.json")?
                    .clone();
                WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
