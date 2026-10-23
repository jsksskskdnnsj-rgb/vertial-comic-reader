#!/usr/bin/env node
// Patches src-tauri/src/main.rs for v0.6 (run from the repo root). Idempotent; aborts WITHOUT writing if an anchor
// is missing or ambiguous (your main.rs then differs from the pasted v0.5.x text: send me the file).
//   1 ThemeConfig gets `banner` (serde default "default": old theme-config.json files still load)
//   2 new commands: set_banner_image(src), reset_banner_image(), get_default_data_dir()   (72 commands in total)
//   3 the three commands are added to generate_handler!
//   4 set_data_dir also migrates theme-config.json (hero + banner paths rewritten), the two custom image files and covers/
// Not compiled by me (no Rust toolchain in my environment): run `cargo check` in src-tauri afterwards.
import fs from 'node:fs'; import path from 'node:path';
const P = path.join(process.cwd(), 'src-tauri', 'src', 'main.rs');
if (!fs.existsSync(P)) { console.error('src-tauri/src/main.rs not found (run from the repo root)'); process.exit(2); }
let raw = fs.readFileSync(P, 'utf8'); const crlf = raw.includes('\r\n'); let s = raw.replace(/\r\n/g, '\n');
if (s.includes('fn set_banner_image')) { console.log('main.rs already patched'); process.exit(0); }

const BANNER_FIELD = `    /// "default" 或自定义首页横幅绝对路径（数据目录下的 banner-custom.<ext>），与整体背景 hero 相互独立
    #[serde(default = "default_banner")]
    banner: String,
`;
const NEW_FNS = `/// 把用户选的图复制为数据目录下的首页横幅（banner-custom.<ext>），返回目标路径
#[tauri::command]
fn set_banner_image(src: String) -> Result<String, String> {
    let dir = data_dir()?;
    let ext = PathBuf::from(&src)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "jpg".to_string());
    let dest = dir.join(format!("banner-custom.{}", ext));
    // 先复制到临时文件再替换旧横幅：选的图本身就是当前横幅时不会被先删掉
    let tmp = dir.join(format!("banner-custom.tmp-{}", now_secs()));
    std::fs::copy(&src, &tmp).map_err(|e| e.to_string())?;
    clear_banner_files(&dir);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}
fn clear_banner_files(dir: &PathBuf) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("banner-custom.") && !n.starts_with("banner-custom.tmp-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}
#[tauri::command]
fn reset_banner_image() -> Result<(), String> {
    clear_banner_files(&data_dir()?);
    Ok(())
}
/// 默认数据目录（exe 所在目录）。前端用它判断当前是否为默认位置，并作为"恢复默认位置"的目标：
/// set_data_dir 选中 exe 目录即删除 data-dir.txt、恢复默认
#[tauri::command]
fn get_default_data_dir() -> Result<String, String> {
    Ok(exe_dir()?.to_string_lossy().to_string())
}
`;
const MIGRATE_FN = `/// 迁移一个主题资源（hero/banner 自定义图）：把文件复制到新目录（已存在则不覆盖），返回新路径
fn migrate_theme_asset(old_dir: &Path, new_dir: &Path, p: &mut String) -> Result<(), String> {
    if p.as_str() == "default" {
        return Ok(());
    }
    if let Some(name) = Path::new(p.as_str()).file_name() {
        let old_f = old_dir.join(name);
        let new_f = new_dir.join(name);
        if old_f.exists() && !new_f.exists() {
            std::fs::copy(&old_f, &new_f).map_err(|e| format!("迁移图片失败：{}", e))?;
        }
        *p = new_f.to_string_lossy().to_string();
    }
    Ok(())
}
`;
const MIGRATE_BLOCK = `
        // 主题配置 + 自定义背景图/横幅（路径改指向新目录）
        let old_theme = old_dir.join("theme-config.json");
        let new_theme = new_dir.join("theme-config.json");
        if old_theme.exists() && !new_theme.exists() {
            let mut cfg: ThemeConfig = std::fs::read_to_string(&old_theme)
                .ok()
                .and_then(|s| serde_json::from_str::<ThemeConfig>(&s).ok())
                .unwrap_or_default();
            migrate_theme_asset(&old_dir, &new_dir, &mut cfg.hero)?;
            migrate_theme_asset(&old_dir, &new_dir, &mut cfg.banner)?;
            let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&new_theme, s).map_err(|e| format!("迁移主题配置失败：{}", e))?;
        }
        // 封面缓存（可再生，复制只为省去重新生成）
        copy_dir_recursive(&old_dir.join("covers"), &new_dir.join("covers"))?;
`;
const steps = [
  ['ThemeConfig.banner field', /(^[ \t]*hero: String,[^\n]*\n)(\})/m, (m, a, b) => a + BANNER_FIELD + b],
  ['default_banner fn', /^impl Default for ThemeConfig \{/m, (m) => 'fn default_banner() -> String {\n    "default".to_string()\n}\n' + m],
  ['Default impl banner', /(^[ \t]*hero: "default"\.to_string\(\),\n)/m, (m, a) => a + '            banner: default_banner(),\n'],
  ['new commands', /^#\[tauri::command\]\nfn pick_data_dir\(/m, (m) => NEW_FNS + m],
  ['register commands', /^([ \t]*)set_data_dir,\n/m, (m, ind) => m + `${ind}set_banner_image,\n${ind}reset_banner_image,\n${ind}get_default_data_dir,\n`],
  ['migrate helper', /^\/\/\/ 切换数据目录/m, (m) => MIGRATE_FN + m],
  ['migrate block', /(copy_dir_recursive\(&old_cache, &new_cache\)\?;\n[ \t]*\}\n)/, (m, a) => a + MIGRATE_BLOCK.replace(/\n$/, '\n')],
];
let out = s;
for (const [name, re, fn] of steps) {
  const hits = out.match(new RegExp(re.source, re.flags.includes('g') ? re.flags : re.flags + 'g'));
  if (!hits || hits.length !== 1) { console.error(`ANCHOR ${hits ? 'AMBIGUOUS (' + hits.length + ')' : 'NOT FOUND'}: ${name}. Nothing written.`); process.exit(1); }
  out = out.replace(re, fn);
}
fs.writeFileSync(P, crlf ? out.replace(/\n/g, '\r\n') : out);
console.log('main.rs patched: ThemeConfig.banner, set_banner_image, reset_banner_image, get_default_data_dir, set_data_dir migration. Run cargo check.');
