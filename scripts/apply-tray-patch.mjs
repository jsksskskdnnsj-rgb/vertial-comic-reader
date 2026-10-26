#!/usr/bin/env node
// v0.6.4 backend + reader patch. Run from the REPO ROOT:  node scripts/apply-tray-patch.mjs
// Edits three files with exact-text anchors and writes NOTHING unless every anchor is found exactly once (idempotent, CRLF kept):
//   src-tauri/src/main.rs   tray icon + menu (Show / Quit), close-to-tray, 3 new commands, ReaderConfig.close_to_tray
//   src-tauri/Cargo.toml    adds the "tray-icon" feature to the tauri dependency
//   src/reader-boot.js      the reader's own close handler must hide to the tray instead of destroying the window
// NOT compiled by me (no Rust toolchain in my environment): run `cd src-tauri && cargo check` afterwards.
import fs from 'node:fs'; import path from 'node:path';
const ROOT = process.cwd();
const FILES = { main: path.join(ROOT, 'src-tauri', 'src', 'main.rs'), cargo: path.join(ROOT, 'src-tauri', 'Cargo.toml'), boot: path.join(ROOT, 'src', 'reader-boot.js') };
const read = (p) => { const raw = fs.readFileSync(p, 'utf8'); return { crlf: raw.includes('\r\n'), s: raw.replace(/\r\n/g, '\n') }; };
const fail = (m) => { console.error(m + '\nNothing written.'); process.exit(1); };
for (const [k, p] of Object.entries(FILES)) if (!fs.existsSync(p)) fail(`${path.relative(ROOT, p)} not found (run from the repo root)`);

const HELPERS = `// ---- v0.6.4: system tray, close-to-tray, window drag, real exit ----
// Hiding to the tray is only allowed once the tray icon exists (otherwise a hidden window could never be brought back).
static TRAY_OK: AtomicBool = AtomicBool::new(false);
fn hide_to_tray_enabled() -> bool {
    TRAY_OK.load(Ordering::SeqCst) && get_reader_config().map(|c| c.close_to_tray).unwrap_or(false)
}
fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    let w = app.get_webview_window("main").or_else(|| app.webview_windows().into_values().next());
    if let Some(w) = w {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
fn init_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    // no icon -> no tray, and hide-to-tray stays off (see TRAY_OK)
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => return Ok(()),
    };
    let show = MenuItem::with_id(app, "tray_show", "显示 / Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出 / Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    #[allow(deprecated)] // renamed show_menu_on_left_click in tauri 2.2; the old name compiles on every 2.x
    let builder = TrayIconBuilder::with_id("mh-tray")
        .icon(icon)
        .tooltip("MH Reader")
        .menu(&menu)
        .menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_show" => show_main_window(app),
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    builder.build(app)?;
    TRAY_OK.store(true, Ordering::SeqCst);
    Ok(())
}
/// 拖动窗口（自定义标题栏按下鼠标时调用）
#[tauri::command]
fn win_start_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}
/// 关闭到托盘开启时隐藏窗口并返回 true；否则返回 false（调用方自行退出）。阅读页用它代替 destroy()
#[tauri::command]
fn win_hide_if_tray(window: tauri::Window) -> Result<bool, String> {
    if hide_to_tray_enabled() {
        window.hide().map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}
/// 真正退出程序（设置页"立即关闭程序"用；win_close 在关闭到托盘时只会隐藏窗口）
#[tauri::command]
fn app_exit(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}
`;
const DEFAULT_FN = `fn default_close_to_tray() -> bool {
    true
}
`;
const WINDOW_EVENT = `.on_window_event(|window, event| match event {
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                let _ = push_drop_queue(paths);
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if hide_to_tray_enabled() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            _ => {}
        })`;
const TRAY_SETUP = `            // v0.6.4 system tray; a failure here must not stop the app, it only keeps close-to-tray off
            if let Err(e) = init_tray(app) {
                eprintln!("tray: {}", e);
            }
`;
const MAIN_STEPS = [
  ['import atomics', /^use std::sync::Mutex;\n/m, (m) => m + 'use std::sync::atomic::{AtomicBool, Ordering};\n'],
  ['ReaderConfig.close_to_tray field', /(^[ \t]*mode: String,[^\n]*\n)(\})/m, (m, a, b) => a + '    #[serde(default = "default_close_to_tray")]\n    close_to_tray: bool,\n' + b],
  ['default_close_to_tray fn', /^impl Default for ReaderConfig \{/m, (m) => DEFAULT_FN + m],
  ['ReaderConfig::default', /mode: "scroll"\.to_string\(\) \}/, () => 'mode: "scroll".to_string(), close_to_tray: default_close_to_tray() }'],
  ['tray helpers + commands', /^fn main\(\) \{/m, (m) => HELPERS + '\n' + m],
  ['tray in setup', /(app\.manage\(cover_protocol::CoversDir\(std::sync::Arc::new\(\n[ \t]*dir\.join\("covers"\),\n[ \t]*\)\)\);\n)([ \t]*Ok\(\(\)\))/, (m, a, b) => a + TRAY_SETUP + b],
  ['window event', /\.on_window_event\(\|_window, event\| \{\n[ \t]*if let tauri::WindowEvent::DragDrop\(tauri::DragDropEvent::Drop \{ paths, \.\. \}\) = event \{\n[ \t]*let _ = push_drop_queue\(paths\);\n[ \t]*\}\n[ \t]*\}\)/, () => WINDOW_EVENT],
  ['register commands', /^([ \t]*)win_close,\n/m, (m, ind) => m + `${ind}win_start_drag,\n${ind}win_hide_if_tray,\n${ind}app_exit,\n`],
];
const BOOT_OLD = 'win.onCloseRequested(async (e) => { e.preventDefault(); auto.stop(); await view?.close(); await win.destroy(); });';
const BOOT_NEW = `win.onCloseRequested(async (e) => {
  e.preventDefault(); auto.stop();
  // close-to-tray: hide the window and keep the book open (progress is flushed when the page becomes hidden); otherwise quit as before
  if (await invoke('win_hide_if_tray').catch(() => false)) return;
  await view?.close(); await win.destroy();
});`;

// ---- main.rs
const main = read(FILES.main); let ms = main.s, mainDone = ms.includes('fn win_start_drag');
if (!mainDone) for (const [name, re, fn] of MAIN_STEPS) {
  const hits = ms.match(new RegExp(re.source, re.flags.includes('g') ? re.flags : re.flags + 'g'));
  if (!hits || hits.length !== 1) fail(`main.rs: anchor ${hits ? 'AMBIGUOUS (' + hits.length + ')' : 'NOT FOUND'}: ${name}`);
  ms = ms.replace(re, fn);
}
// ---- Cargo.toml
const cargo = read(FILES.cargo); let cs = cargo.s, cargoDone = false;
{
  const m = cs.match(/^(tauri\s*=\s*)(.+)$/m);
  if (!m) fail('Cargo.toml: no `tauri = ...` line (a [dependencies.tauri] table?). Add the "tray-icon" feature to tauri by hand.');
  let val = m[2].replace(/\s+#.*$/, '').trim();
  if (/tray-icon/.test(val)) cargoDone = true;
  else {
    let nv;
    if (/^"[^"]*"$/.test(val)) nv = `{ version = ${val}, features = ["tray-icon"] }`;
    else if (/^\{.*\}$/.test(val)) nv = /features\s*=\s*\[/.test(val)
      ? val.replace(/features\s*=\s*\[([^\]]*)\]/, (mm, inner) => `features = [${inner.trim() ? inner.trim().replace(/,\s*$/, '') + ', ' : ''}"tray-icon"]`)
      : val.replace(/\s*\}$/, ', features = ["tray-icon"] }');
    else fail('Cargo.toml: unrecognised tauri dependency form: ' + m[2]);
    cs = cs.replace(m[0], m[1] + nv);
  }
}
// ---- reader-boot.js
const boot = read(FILES.boot); let bs = boot.s, bootDone = bs.includes('win_hide_if_tray');
if (!bootDone) {
  if (bs.split(BOOT_OLD).length !== 2) fail('reader-boot.js: the close handler line was not found exactly once (your copy differs from batch 1).');
  bs = bs.replace(BOOT_OLD, () => BOOT_NEW);
}
const out = (o, s) => (o.crlf ? s.replace(/\n/g, '\r\n') : s);
if (!mainDone) fs.writeFileSync(FILES.main, out(main, ms));
if (!cargoDone) fs.writeFileSync(FILES.cargo, out(cargo, cs));
if (!bootDone) fs.writeFileSync(FILES.boot, out(boot, bs));
console.log(`main.rs: ${mainDone ? 'already patched' : 'patched'}; Cargo.toml: ${cargoDone ? 'already has tray-icon' : 'tray-icon added'}; reader-boot.js: ${bootDone ? 'already patched' : 'patched'}`);
console.log('Next: (cd src-tauri && cargo check), then run the app and follow the on-device list in RUST-TRAY-PATCH.md');
