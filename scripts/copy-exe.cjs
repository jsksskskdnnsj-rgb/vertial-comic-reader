// 构建完成后把二进制复制到工程根目录（第一层），方便直接运行，
// 数据库也会默认保存在它旁边。跨平台：Windows 取 mh-reader.exe，其它平台取 mh-reader。
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const name = 'mh-reader' + (process.platform === 'win32' ? '.exe' : '');
const src = path.join(root, 'src-tauri', 'target', 'release', name);
const dest = path.join(root, name);

if (!fs.existsSync(src)) {
  console.error('[copy-exe] 找不到 ' + src + '，构建可能失败了');
  process.exit(1);
}
fs.copyFileSync(src, dest);
if (process.platform !== 'win32') {
  try { fs.chmodSync(dest, 0o755); } catch (e) { /* 忽略 */ }
}
console.log('[copy-exe] 已复制到 ' + dest);
