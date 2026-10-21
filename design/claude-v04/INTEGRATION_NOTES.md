# v0.4 Integration Notes — mapping & merge decisions

Status: integration working document for mh-reader v0.4.0. Sources: the 15 design
files in this folder (Claude's cumulative merge) and the v0.3.x project
(`src/index.html`, `src/reader.html`, `src/reader-boot.js`, `src/mh-common.js`,
`src-tauri/src/main.rs`). Claude's later canonical set, when it arrives, is a
cross-check only; the decisions below win on conflict.

## 1. File mapping (design → project)

| Design file | Project target | Action |
|---|---|---|
| `home.html` | `src/home.html` | New app entry page. |
| `home.js` | `src/home.js` | New. |
| `mh-ui.css` | `src/mh-ui.css` | New design system; replaces inline styles of old pages. |
| `icons.svg` | `src/icons.svg` | New. All 21 referenced `<use>` ids verified defined. |
| `shell.js` | `src/shell.js` | Replaces `mh-common.js` role for new pages (ES module, `window.__TAURI__`). `mh-common.js` stays in `src/` untouched — the legacy pages need it. |
| `reader.html` | `src/reader.html` | **Replaces** existing `src/reader.html`. Old file renamed `src/legacy-reader.html`. |
| `reader-boot.js` | `src/reader-boot.js` | **Replaces** existing. Old file renamed `src/legacy-reader-boot.js`. |
| `reader.js` | `src/reader.js` | New reader engine (was embedded in old `reader.html`). |
| `layout.js` | `src/layout.js` | New. Required by `reader.js` (user-found trap). |
| `chapters.js` | `src/chapters.js` | New. Required by `reader-boot.js` (user-found trap; missing from Step C zip, present in the merged design set). |
| `bookmarks.js` | `src/bookmarks.js` | New. |
| `chapter-settings.html` | embedded in `src/settings.html` | Snippet, not a page: its `<section id="chapterDefaults">` + inline logic are folded into the new settings page. The file itself also stays in `src/` for reference parity with the design set. |
| — | `src/library.html` (+`library.js`) | **New, written during integration** (§4). |
| — | `src/import.html` (+`import.js`) | **New, written during integration** (§4). |
| — | `src/settings.html` (+`settings.js`) | **New, written during integration** (§4). |
| `cover_protocol.rs` | `src-tauri/src/cover_protocol.rs` | Module of the backend crate; `ensure_cover` internals adapted (§3.4). |
| `reader_backend.rs` | `src-tauri/src/{error,model,scan,db,cache,extractor,protocol,commands,chapters,bookmarks}.rs` | Split per author's header instruction; adapted to the existing `shelf` table (§3). |
| `README.md` (design) | stays in `design/claude-v04/` only | Not copied to `src/`. |
| existing `src/index.html` | `src/legacy.html` | Renamed (not deleted): keeps the v0.3.x single-page app available for reference/rollback, and removes any chance Tauri serves it as the default entry. Cleaner than leaving a stale `index.html` next to `home.html`. |
| existing `src-tauri/src/main.rs` | stays, extended | All 32 existing commands/wiring preserved; new modules + commands + protocols registered alongside. |

Window entry: `src-tauri/tauri.conf.json` `app.windows[0]` gains `"url": "home.html"`.
Reader links everywhere use `reader.html?book=<shelf_id>` (design boot reads
`?book=`, **not** the old `?id=`).

## 2. Command contract

### 2.1 Existing commands — all kept, unchanged signatures
`pick_images`, `pick_zips`, `pick_folder_paths`, `list_images`, `zip_open`,
`zip_extract_page` (rename_all snake_case), `clear_cache`, `open_data_dir`,
`shelf_add`, `shelf_list`, `shelf_get`, `shelf_progress`, `shelf_remove`,
`shelf_clear`, `get_data_dir`, `pick_data_dir`, `set_data_dir`,
`get_reader_config`, `set_reader_config`, `get_theme_config`, `set_theme_config`,
`set_hero_image`, `reset_hero_image`, `win_minimize`, `win_toggle_maximize`,
`win_close`, `take_drop_queue`, `folder_add`, `folder_list`, `folder_rename`,
`folder_remove`, `shelf_move` (rename_all snake_case).

### 2.2 New commands from the design backend (ported)
Reader core: `open_book`, `rescan_book`, `close_book`, `set_reading_window`,
`save_progress`, `load_progress`, `fix_page_dims`, `cache_stats`,
`cache_set_limit`, `cache_clear`.
Chapters: `get_chapter_defaults`, `set_chapter_defaults`,
`set_book_chapter_settings`, `clear_book_chapter_settings`, `episode_info`.
Bookmarks: `add_bookmark`, `list_bookmarks`, `delete_bookmark`,
`update_bookmark_note`, `reanchor_bookmark`, `bookmark_counts`.
Covers: `ensure_cover` + `mhcover` protocol. Pages: `mhpage` protocol.
(`load_progress`/`bookmark_counts`/`rescan_book`/cache commands have no
frontend caller in the design; they are ported anyway — the backend provides
them and the settings page uses the cache trio.)

### 2.3 Adapter commands — designed here, implemented in the merge
The design frontend calls these but the design package has no backend for them:

- `list_recent_books({limit}) -> [{id, title, pageCount, page, frac, lastReadAt, coverVer}]`
  from `shelf` ordered by `updated_at DESC`, restricted to books with reading
  progress (`progress` row or `last_page > 0`); `title = shelf.name`,
  `page/frac` from `progress` (fallback `last_page`, `0`), `lastReadAt = updated_at * 1000`.
- `list_recent_added({limit}) -> [{id, title, pageCount, addedAt, coverVer}]`
  from `shelf` ordered by `created_at DESC`; `addedAt = created_at * 1000`.
- `import_paths({paths, folderId}) -> {added, skipped, failed: [{path, reason}]}`
  Rust-side import mirroring the verified frontend pipeline of v0.3.x:
  zip/cbz path → shelf row (page_count = image entries, filtered with an
  extension list mirroring `MH.imgRe` — broader than `IMAGE_EXTS`, incl.
  `jfif`); folder path → `walk_dir` listing as pages; loose image files →
  accumulated into ONE `images` book (name `<first stem> 等<N>张`,
  `total_size: 0`, as before). Exact `(kind, paths)` duplicates count as
  `skipped`. `folderId` non-null sets `shelf.folder_id`. After a successful
  insert, cover generation is triggered (same code path as `ensure_cover`).
- `reveal_book({bookId})` — open the book's source location in Explorer
  (Windows `explorer /select,` for zip/images-first-file; the folder itself
  for folder books); no-op-Ok off Windows, mirroring `open_data_dir`.
- `relink_book({bookId, path})` — for zip/folder books replace `paths[0]`,
  set `scan_sig = NULL` so the next `open_book` rescans; returns `()`.

## 3. Data model unification (the core merge decision)

The design backend assumes a `books` table; the existing app has a verified
`shelf` table. **Decision: `shelf` is the book table.** The design's `db` /
`commands` layer is adapted to it; no second book table is created.

| Design `books` field | Maps to |
|---|---|
| `id` | `shelf.id` (book_id everywhere = shelf id) |
| `title` | `shelf.name` |
| `kind` | `shelf.kind` (`zip`/`folder`/`images` → `SourceKind`) |
| `path` | `shelf.paths` JSON: `[0]` for zip/folder; the full array for `images` |
| `page_count` | `shelf.page_count` (scan refreshes it) |
| `last_read_at` | `shelf.updated_at` (unix secs) |
| `scan_sig` | **new column** `shelf.scan_sig TEXT` (migration, §3.2) |
| `finished` | **new column** `shelf.finished INTEGER NOT NULL DEFAULT 0` |
| `cover_ver` | **new column** `shelf.cover_ver INTEGER NOT NULL DEFAULT 1`; bumped by `ensure_cover` after (re)generating a cover file |

New tables in the same `mh-reader.db` (design DDL, run at startup in this
order after existing migrations): `pages_meta`, `progress` (authoritative
reader position: `page` + `frac`), `settings`, `book_settings`, `bookmarks`
(+ `idx_bookmarks_book_entry`, `idx_pages_meta_name`). The design page cache
keeps its own DB at `data_dir/cache/cache.db` (design `PageCache`), distinct
from the existing `page_cache/` folder (still used by `zip_extract_page`).

Progress mirroring: design `save_progress` writes the `progress` table AND
mirrors `shelf.last_page = pos.page`, `shelf.updated_at = now` (preserves
v0.3.x "recently read" ordering and the legacy field). `open_book`'s `resume`
comes from `progress`, falling back to `{page: last_page, frac: 0}`.

Loose-image books: the design's `open_blocking` returns `Unsupported` for
`SourceKind::Images` — during integration this branch is implemented using
the `shelf.paths` array (the data exists; `scan::scan_images` already handles
it). `quick_sig` for image books stays the design's constant (source changes
to a loose-image set are not auto-detected; acceptable, noted).

Locking: the design holds one `Arc<Mutex<Connection>>` for the app lifetime
while existing commands open per-call connections to the same file. Both sides
get `PRAGMA busy_timeout = 5000` (existing `db_conn()` included — behaviour-
preserving, only affects contention waits).

### 3.1 Migrations
Existing pattern is kept (CREATE IF NOT EXISTS + `pragma_table_info` checks in
`db_conn()`); the design DDL/migrations run once in `setup` against the same
DB file, guarded the same way (`scan_sig` has no `IF NOT EXISTS` in the
design DDL — guard it like `folder_id`).

### 3.2 Window / capabilities / config decisions
- `decorations: false` stays. `transparent: true` is set per the design
  `shell.js` header (the design CSS paints the full window surface).
- CSP stays `null` (existing). The design's suggested CSP would block the
  settings page's inline module script and is a behaviour change; recorded as
  a deliberate deviation.
- `assetProtocol` stays enabled with scope `["**"]` (existing; legacy pages
  and `reveal`/cover fallbacks rely on the machinery being present).
- A capabilities file **is** added — `src-tauri/capabilities/default.json` —
  because the design frontend uses `window.__TAURI__.window`,
  `T.dialog.open`, `T.webview.onDragDropEvent` and `T.app.getVersion`
  directly (the v0.3.x frontend avoided these APIs, which is why none existed).
  It grants only what `shell.js`'s header lists plus `core:app:allow-version`
  and `core:default`. Custom `win_*` commands remain for the legacy pages.
- Reader display prefs (width/gap/bg/dim/saver) and theme/lang live in
  `localStorage` (`mh.reader`, `mh.theme`, `mh.lang`) — the design's native
  store. `reader-config.json`/`theme-config.json` commands remain in the
  backend for compatibility; the new settings page writes localStorage for
  display prefs and uses the JSON commands only where they are the existing
  contract (none of the new pages depend on them).
- The design reader is scroll-only: v0.3.x's `mode: "page"` is not carried
  into the new reader engine. Recorded as a known v0.4 regression-by-design.

### 3.3 Covers
`mhcover` serves persisted files `data_dir/covers/{book_id}.jpg`; the serve
handler is ported as-is. `ensure_cover`'s **internals are rewritten** to use
the verified machinery instead of the design extractor (which it otherwise
drags in just for covers): zip → `zip_open` + first image entry +
`zip_extract_page`; folder → first image of `list_images`; images →
`paths[0]`. Decode + downscale to 240 px wide JPEG with the `image` crate,
atomic write (`.part` + rename), then bump `shelf.cover_ver`. This keeps the
design's URL/404/retry contract with `home.js` intact.

### 3.4 Design-suggested backend fixes applied during the port
`model::VARIANTS` gains 240 → `[0, 160, 240, 800, 1200, 1600, 2400]`;
`commands::open_blocking` becomes `pub(crate)`; rusqlite stays at the
project's **0.32** (design was written against 0.31 — API-compatible for the
used surface, `cargo check` is the arbiter). New Cargo deps from the design
header: `thiserror 1`, `parking_lot 0.12`, `imagesize 0.13`, `natord 1`,
`walkdir 2`, `http 1`, `image 0.25` (default-features off; jpeg/png/webp/gif/bmp).

## 4. The three new pages (written during integration)

All reuse `shell.js` + `mh-ui.css` + `icons.svg` and the shared sidebar/topbar
markup from `home.html`. Navigation ids in links are shelf ids; reader links
use `reader.html?book=<id>`.

- `library.html`/`library.js` — full shelf from `shelf_list` (+ `folder_list`
  sidebar filter incl. 未分类）, search box honours `?q=` (home search target)
  and `?sort=recent|added`. Each card: cover via `mhcover` URL (404 →
  `ensure_cover` retry, same pattern as home), title, episode label via
  `chapters.js fetchEpisodeInfo`/`episode_info` batch, bookmark ribbon/count
  via `bookmark_counts`, click → reader; row menu: reveal, move to folder
  (`shelf_move`), remove (`shelf_remove`).
- `import.html`/`import.js` — pickers (`pick_images`/`pick_zips`/
  `pick_folder_paths`) + whole-window drag-drop via
  `T.webview.onDragDropEvent`, all funnelled into `import_paths`; result
  summary (added/skipped/failed with reasons) listed on the page.
  (The Rust drop-queue still records drops for the legacy page; the new
  pages consume drops through the webview event and do not poll the queue —
  a stale `drop-queue.json` can therefore accumulate while using only the
  new UI. Known wart, accepted for v0.4; the queue is drained by
  `legacy.html` if ever opened.)
- `settings.html`/`settings.js` — sections: Reading (display defaults →
  `localStorage mh.reader`; the `chapterDefaults` section from
  `chapter-settings.html` → `get/set_chapter_defaults`), Theme (→
  `localStorage mh.theme` via shell.js `applyTheme` conventions), Data
  (current dir from `get_data_dir`; switch via `pick_data_dir` +
  `set_data_dir` — the preserved copy-on-switch flow; open dir via
  `open_data_dir`), Cache (design cache: `cache_stats`/`cache_set_limit`/
  `cache_clear`; legacy page cache: `clear_cache`), About (version via
  `T.app.getVersion`, clear-shelf danger zone via `shelf_clear`).
