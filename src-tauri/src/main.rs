#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// v0.4 reader backend (split from design/claude-v04/reader_backend.rs; see
// design/claude-v04/INTEGRATION_NOTES.md §3 for the shelf-unification decisions).
// App shell pieces that used to live in this file are now paths / appdata /
// config / shelf / window; main only wires plugins, protocols, and commands.
mod adapters;
mod archives;
mod appdata;
mod backup;
mod bookmarks;
mod cache;
mod chapters;
mod commands;
mod config;
mod cover_protocol;
mod db;
mod error;
mod extractor;
mod library;
mod merge;
mod metadata;
mod model;
mod paths;
mod protocol;
mod scan;
mod shelf;
mod shelf_scan;
mod tags;
mod window;

use rusqlite::Connection;
use tauri::Manager;

// Re-export the helpers other modules still call as `crate::…`.
pub(crate) use appdata::{data_dir, db_conn, now_secs};
pub(crate) use paths::{ext_of, kind_of, list_images, DECODE_EXTS, IMAGE_EXTS};

fn main() {
    appdata::install_panic_log();
    tauri::Builder::default()
        // Must be the first plugin. A second launch is handed to this process,
        // which unminimizes and focuses the window already on screen.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main_window(&app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // v0.4 reader backend wiring (design reader_backend.rs wiring block,
            // adapted to this crate's data_dir()/db_conn()).
            app.manage(shelf_scan::ScanHost::default());
            let dir = data_dir().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            // db_conn() runs the base DDL + shelf column migrations (incl. the new
            // scan_sig/finished/cover_ver columns the design DDL expects).
            let _ = db_conn().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let conn = Connection::open(dir.join("mh-reader.db"))?;
            conn.busy_timeout(std::time::Duration::from_millis(5000))?;
            // Design migrations once, in order: db DDL -> chapters DDL -> bookmarks
            // (bookmarks::migrate also runs bookmarks::DDL). All idempotent.
            conn.execute_batch(db::DDL)?;
            conn.execute_batch(db::EDITED_DDL)?;
            conn.execute_batch(chapters::DDL)?;
            bookmarks::migrate(&conn)?;
            metadata::store::migrate(&conn).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let rdb: db::Db = std::sync::Arc::new(parking_lot::Mutex::new(conn));
            let cache =
                std::sync::Arc::new(cache::PageCache::open(dir.join("cache"), 512 << 20)?);
            let extractor = extractor::Extractor::start(cache.clone(), extractor::worker_count());
            app.manage(commands::ReaderState {
                db: rdb,
                cache,
                extractor,
            });
            let covers = dir.join("covers");
            cover_protocol::trim_covers(&covers, 512 << 20);
            app.manage(cover_protocol::CoversDir(std::sync::Arc::new(covers)));
            // v0.6.4 system tray; a failure here must not stop the app, it only keeps close-to-tray off
            if let Err(e) = window::init_tray(app) {
                eprintln!("tray: {}", e);
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("mhpage", |ctx, req, responder| {
            let ex = ctx
                .app_handle()
                .state::<commands::ReaderState>()
                .extractor
                .clone();
            protocol::handle(ex, req, responder);
        })
        .register_asynchronous_uri_scheme_protocol("mhcover", |ctx, req, responder| {
            let dir = ctx
                .app_handle()
                .state::<cover_protocol::CoversDir>()
                .0
                .clone();
            cover_protocol::cover_handle(dir, req, responder);
        })
        .register_asynchronous_uri_scheme_protocol("mhfile", |_ctx, req, responder| {
            tauri::async_runtime::spawn(async move {
                let resp = tauri::async_runtime::spawn_blocking(move || protocol::serve_theme_file(&req))
                    .await
                    .unwrap_or_else(|_| {
                        http::Response::builder().status(500).body(Vec::new()).unwrap()
                    });
                responder.respond(resp);
            });
        })
        .invoke_handler(tauri::generate_handler![
            paths::pick_folder_paths,
            paths::list_images,
            shelf_scan::scan_begin,
            shelf_scan::scan_status,
            shelf_scan::scan_cancel,
            shelf_scan::list_scan_issues,
            appdata::open_data_dir,
            shelf::shelf_list,
            shelf::shelf_get,
            shelf::shelf_remove,
            shelf::shelf_clear,
            appdata::get_data_dir,
            appdata::pick_data_dir,
            appdata::set_data_dir,
            config::set_banner_image,
            config::reset_banner_image,
            appdata::get_default_data_dir,
            config::get_reader_config,
            config::set_reader_config,
            config::get_theme_config,
            config::set_theme_config,
            config::set_hero_image,
            config::reset_hero_image,
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_close,
            window::win_start_drag,
            window::win_hide_if_tray,
            window::app_exit,
            window::take_drop_queue,
            shelf::folder_add,
            shelf::folder_list,
            shelf::folder_rename,
            shelf::folder_remove,
            shelf::folder_reorder,
            shelf::shelf_move,
            // v0.4 reader core (design backend)
            commands::open_book,
            commands::rescan_book,
            commands::close_book,
            commands::set_reading_window,
            commands::save_progress,
            commands::load_progress,
            commands::fix_page_dims,
            commands::cache_stats,
            commands::cache_set_limit,
            commands::cache_clear,
            // v0.4 chapters
            chapters::get_chapter_defaults,
            chapters::set_chapter_defaults,
            chapters::set_book_chapter_settings,
            chapters::clear_book_chapter_settings,
            chapters::episode_info,
            // v0.4 bookmarks
            bookmarks::add_bookmark,
            bookmarks::list_bookmarks,
            bookmarks::delete_bookmark,
            bookmarks::update_bookmark_note,
            bookmarks::reanchor_bookmark,
            bookmarks::bookmark_counts,
            // v0.4 covers
            cover_protocol::ensure_cover,
            // v0.4 adapters (INTEGRATION_NOTES §2.3)
            adapters::list_recent_books,
            adapters::list_recent_added,
            adapters::import_paths,
            adapters::import_scan_books,
            merge::merge_auto_get,
            merge::merge_auto_set,
            merge::list_merge_pending,
            merge::resolve_merge_pending,
            merge::merge_books,
            merge::unmerge_book,
            adapters::reveal_book,
            adapters::relink_book,
            // v0.5 library management + page edits (library.rs)
            library::reorder_pages,
            library::delete_pages,
            library::append_pages,
            library::reset_page_edits,
            library::page_edit_info,
            library::shelf_rename,
            library::book_reset_progress,
            library::check_books_missing,
            library::import_merged,
            library::library_stats,
            tags::list_tags,
            tags::tag_create,
            tags::tag_rename,
            tags::tag_delete,
            tags::book_set_tags,
            tags::book_set_favorite,
            tags::books_tags_bulk,
            tags::find_duplicate_books,
            backup::export_library_backup,
            backup::import_library_backup,
            metadata::commands::metadata_get_config,
            metadata::commands::metadata_set_config,
            metadata::commands::metadata_search,
            metadata::commands::metadata_detail,
            metadata::commands::metadata_get,
            metadata::commands::metadata_list_pending,
            metadata::commands::metadata_bind,
            metadata::commands::metadata_unbind,
            metadata::commands::metadata_automatch,
            metadata::commands::metadata_refresh_cover,
            metadata::commands::metadata_cover_local_path,
            metadata::commands::metadata_open_source_page,
            metadata::commands::metadata_comic18_search_url,
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                let _ = window::push_drop_queue(paths);
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window::hide_to_tray_enabled() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod image_ext_contract {
    #[test]
    fn dialog_and_decoder_lists_stay_aligned() {
        for ext in crate::DECODE_EXTS {
            assert!(
                crate::IMAGE_EXTS.contains(ext),
                "{ext} is decodable but not importable"
            );
        }
        assert!(crate::IMAGE_EXTS.contains(&"avif"));
        assert!(
            !crate::DECODE_EXTS.contains(&"avif"),
            "avif is passthrough-only until the image crate avif feature is enabled"
        );
        let n = crate::extractor::worker_count();
        assert!((2..=4).contains(&n), "extract workers {n} out of 2..=4");
        for banned in ["svg", "tif", "tiff", "heic", "heif", "jxl", "ico", "apng", "dib"] {
            assert!(!crate::IMAGE_EXTS.contains(&banned), "{banned} must not be imported");
            assert!(!crate::DECODE_EXTS.contains(&banned), "{banned} must not be a cover candidate");
        }
    }
}
