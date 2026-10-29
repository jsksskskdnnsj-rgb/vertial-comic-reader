# MH Comic Reader (Tauri Desktop) v0.7.4

> **Download for Windows**: grab `mh-reader.exe` from [Releases](https://github.com/jsksskskdnnsj-rgb/vertial-comic-reader/releases) — double-click to run, no build needed.
> To build from source, see "Build & Run" below.

A local vertical-scroll comic reader for Windows (Tauri 2 + Rust + vanilla HTML/JS). Frameless window, about 1200×800 by default, with an optional tray icon. The UI is a soft-light glass theme, in Chinese and English.

**Windows 10/11 is the tested platform** (WebView2 ships with the OS). macOS and Linux may build, but they have not been tested.

[中文说明](README.md)

## Features

- **Pages**: Home, Library, Import, book details, Settings, and the reader. The entry point is `src/home.html` (the old single-page `index.html` is gone)
- **Three import types**: ZIP / CBZ, folders (including nested folders), and loose images — or drag files into the window
- **Scan a folder**: pick one directory. Each immediate subfolder that contains images becomes a book named after that folder, and folders inside it become chapters (2 before 10, including Chinese chapter numbers). Review the list, then add what you want. A folder already on the shelf is not added again. The scan runs in the background and can be cancelled; you can keep reading and change pages while it runs. A small progress pill in the corner of every page opens the scan page. When it finishes, the app shows a toast and, if permitted, a system notification with how many books were added, merged, and still waiting to confirm. An unfinished scan of the same folder continues next time, and a finished one starts fresh. Unreadable images are skipped and listed on the page. Archives and whole-drive scans are not part of this yet — import a zip from the Import page
- **Similar folders**: when folder names share a base after volume, chapter, bracket, and punctuation marks are removed, and each one has a distinct order (chapters 1–50 / 51–100, vol 1 / vol 2, 上 / 下, part 1 / part 2), they become one book automatically, in that order. Auto-merge is on by default and can be turned off in Settings. Uncertain groups wait in a list where you can rename, pick a category, drag or nudge the order, or keep them separate. In the library you can select several folders and merge them, and split a merged book back apart. Only the library records change — files are not moved or deleted
- **On-demand reading**: opening a book scans the listing; pages are extracted as you reach them (extractor queue + virtual list). The whole archive is not loaded into memory
- **SQLite library**: stores paths only, never copies your files. The cover is an early decodable page, scaled to 240px
- **Resume**, bookmarks anchored to a specific image, and chapters (none / by folder / every N pages)
- **Reader**: vertical scroll and page-turn, auto-scroll, page directory, width and gap. Press `?` or the info button for the shortcut list
- **Categories**: create, rename, delete, and reorder them in the library sidebar, then filter by one. Import and scan can file a book into a category, and the book page can move it later. All and Uncategorized stay available
- **Library**: covers show reading progress, and each card shows when you last read it. Large shelves render one page at a time (48 books). Search filters as you type; `/` or Ctrl+K focuses the search box
- **Appearance**: soft-light theme, accent color, separate window background and home banner, panel opacity, wallpaper fit, border color
- **Window**: custom title bar (drag to move, double-click to maximize). Closing can hide to the tray. Launching the app again focuses the window that is already open
- Files stay on this machine — nothing is uploaded

### Image formats

jpg, jpeg, jfif, jpe, png, webp, gif, bmp, and **avif (displayed as-is)**.

The webview can paint AVIF bytes directly. This build of the `image` crate has no AVIF decoder, so an AVIF book may have no cover, and tall-image slices or width variants of AVIF pages will fail. svg, tif, and heic are rejected at import. RAR / 7z are not supported — extract to a folder first.

## Build & Run

**Windows** (tested platform):

1. Install Rust (`rustup-init.exe` from https://rustup.rs), Node.js LTS, and VS Build Tools (Desktop development with C++). Windows 10/11 ships WebView2.
2. Clone the repo and, in the project root:
   ```
   python scripts/gen-icons.py   # redraw every icon (png / ico / icns) in the soft-light theme; overwrites existing files
   npm install                   # first time only
   npm run build
   ```
3. The binary is copied to the project root (`mh-reader.exe`). Double-click it. Rebuild only after code changes.

`scripts/apply-*.mjs` are one-shot patch appliers from the v0.6–v0.7 UI work. They are **not** part of `npm run build`. Running them again can overwrite the soft-light theme; `scripts/apply-v07.mjs` puts that block back at the end of the stylesheet.

## Project Layout

```
mh-reader/
  mh-reader.exe        # copied here after build
  mh-reader.db         # library (next to the exe by default; changeable in Settings)
  cache/               # reader page cache
  covers/              # 240px covers
  data-dir.txt         # only present when a custom data dir is set
  src/
    home.html          # entry
    library.html
    import.html
    scan.html          # folder scan
    book.html
    settings.html
    reader.html        # reader (virtual list in reader.js)
    mh-ui.css          # UI styles; the v0.7 soft-light theme is at the end
  archive/legacy-ui/   # frozen v0.3 UI, not included in the app bundle
  src-tauri/
    tauri.conf.json
    src/               # commands, extractor, SQLite, mhpage / mhcover / mhfile
  scripts/
    copy-exe.cjs
    gen-icons.py       # redraws the app icons (32/128/256/512 png, ico, icns)
```

## Notes

- The first build downloads Rust crates and takes a few minutes.
- Frameless window: custom — ▢ ✕ buttons. Drag empty areas of the title bar to move. Closing hides to the tray by default; Settings can quit instead.
- Custom wallpapers and banners are copied into the data directory and served by the `mhfile` protocol, which only allows `hero-custom.*` and `banner-custom.*`. Comic pages use `mhpage`; covers use `mhcover`.

## License

MIT — see [LICENSE](LICENSE). Changelog: [CHANGELOG.md](CHANGELOG.md).
