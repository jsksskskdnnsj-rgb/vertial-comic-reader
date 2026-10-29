# Archived v0.3 UI (not shipped)

These files are the pre-v0.4 single-page shelf (`legacy.html`, formerly `index.html`) and the old reader (`legacy-reader.html` + `legacy-reader-boot.js` + `mh-common.js`).

The running app starts at `src/home.html`. Tauri’s `frontendDist` is `src/`, so this folder is not bundled.

The snapshot still calls commands the current backend no longer registers (`zip_open`, `zip_extract_page`, `shelf_add`, `shelf_progress`, `pick_images`, `pick_zips`, `clear_cache`). Reading goes through `open_book` and the extractor. Do not wire these pages back into `src/`.

Internal links point at each other (`legacy.html` ↔ `legacy-reader.html`) so the archive has no references to the deleted `index.html`.
