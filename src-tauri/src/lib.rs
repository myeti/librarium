mod commands;
mod convert;
mod epub;
mod error;
mod fsutil;
mod logging;
mod menu;
mod models;
mod mtp;
mod net;

use commands::backup::{
    cancel_backup, export_backup, read_backup, restore_backup, reveal_backup, BackupState,
};
use commands::calibre::{
    cancel_calibre_import, detect_calibre_library, import_from_calibre, list_calibre_books,
    CalibreImportState,
};
use commands::config::{get_config, set_config};
use commands::device::{
    cancel_transfer, delete_device_book, detect_device, detect_devices, eject_device,
    get_mtp_status, list_device_books, repair_device_covers, scan_device_library, send_books,
    TransferState,
};
use commands::http::http_get;
#[cfg(debug_assertions)]
use commands::library::inspect_epub;
use commands::library::{
    batch_update_metadata, check_path_accessible, convert_to_epub, create_user, delete_book_cover,
    delete_books, detect_language, download_and_import, ensure_cover, export_book, import_books,
    list_users, rename_book, reset_book_data, reveal_in_explorer, save_cover_from_url,
    scan_library, set_book_cover, update_metadata,
};
use commands::library::{forget_books, library_lock_status, open_log_folder, set_favorite};
use commands::opds::{
    invalidate_opds_cache, opds_qr_svg, opds_status, start_opds, stop_opds, OpdsServerState,
};
use commands::progress::get_device_reading_progress;
use commands::reader::open_for_reading;
use menu::set_menu_language;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First: a second launch hands over to the running app (and focuses
        // its window) instead of starting another instance that would edit
        // the same library index.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // Early, so the other plugins' setup is logged too.
        .plugin(logging::plugin())
        // Opens `<a target="_blank">` links (and ctrl/shift-clicked ones) in
        // the browser, through its own click handler; `reveal_in_explorer`
        // uses its reveal API.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `relaunch()` after an update has been installed.
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            logging::install(app.handle());
            log::info!("Librarium {} starting", app.package_info().version);
            // MTP Kindles: experimental, on unless the setting turns them off.
            let mtp_enabled =
                commands::config::read_flag(app.handle(), commands::config::MTP_ENABLED, true);
            app.state::<mtp::MtpState>().set_enabled(mtp_enabled);
            // The menu bar, macOS only (see menu.rs): English until the
            // frontend calls `set_menu_language`.
            #[cfg(target_os = "macos")]
            menu::install(app.handle());
            Ok(())
        })
        .manage(OpdsServerState::default())
        .manage(TransferState::default())
        .manage(mtp::MtpState::new(
            mtp::platform_backend(),
            commands::device::mtp::connect_hook(),
            std::sync::Arc::new(logging::log_global),
        ))
        .manage(CalibreImportState::default())
        .manage(BackupState::default())
        .invoke_handler(tauri::generate_handler![
            scan_library,
            import_books,
            rename_book,
            update_metadata,
            batch_update_metadata,
            delete_books,
            forget_books,
            set_favorite,
            library_lock_status,
            reset_book_data,
            // The EPUB inspector is a developer tool: debug builds only.
            #[cfg(debug_assertions)]
            inspect_epub,
            list_users,
            create_user,
            ensure_cover,
            detect_device,
            detect_devices,
            get_mtp_status,
            eject_device,
            scan_device_library,
            list_device_books,
            repair_device_covers,
            delete_device_book,
            get_device_reading_progress,
            send_books,
            cancel_transfer,
            reveal_in_explorer,
            open_log_folder,
            detect_language,
            detect_calibre_library,
            list_calibre_books,
            import_from_calibre,
            cancel_calibre_import,
            export_backup,
            read_backup,
            restore_backup,
            cancel_backup,
            reveal_backup,
            check_path_accessible,
            convert_to_epub,
            export_book,
            download_and_import,
            save_cover_from_url,
            set_book_cover,
            delete_book_cover,
            open_for_reading,
            start_opds,
            stop_opds,
            opds_status,
            invalidate_opds_cache,
            opds_qr_svg,
            http_get,
            get_config,
            set_config,
            set_menu_language,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                // Quitting here and opening the library on another computer
                // right away must not warn about "another instance". NAS I/O:
                // on a worker thread, waited for 1.5 s at most.
                commands::library::release_library_locks();
            }
        });
}
