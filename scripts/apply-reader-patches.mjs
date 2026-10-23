#!/usr/bin/env node
// Applies the v0.6 changes to the batch-1 reader files IN PLACE with exact-string anchors.
// Run from the repo root:  node scripts/apply-reader-patches.mjs
// Each patch is skipped if already applied and aborts (writing nothing for that file) if its anchor is
// missing, which means your copy differs from the original batch-1 text: send me that file.
import fs from 'node:fs'; import path from 'node:path';
const SRC = path.join(process.cwd(), 'src');
const PATCHES = {
  'reader-boot.js': [
    { done: "from './shell-ui.js'",
      find: "} from './shell.js';\nimport { Reader, thumbUrl } from './reader.js';",
      repl: "} from './shell.js';\nimport { mountShell, wireCoverFallback } from './shell-ui.js';\nimport { Reader, thumbUrl } from './reader.js';" },
    { done: "mountShell('library')",
      find: "bootPage('library');",
      repl: "bootPage('library');\nmountShell('library');   // fills #shell-top/#shell-rail, applies accent + background\nwireCoverFallback();" },
  ],
  'reader-tools.js': [
    { done: "from './merge-order.js'", find: "import { sortSources } from './naming.js';", repl: "import { orderByRange } from './merge-order.js';" },
    { done: "orderByRange(picked)", find: "sortSources(picked)", repl: "orderByRange(picked)" },
  ],
};
let bad = 0;
for (const [file, list] of Object.entries(PATCHES)) {
  const p = path.join(SRC, file);
  if (!fs.existsSync(p)) { console.log(`MISSING ${file}`); bad++; continue; }
  let s = fs.readFileSync(p, 'utf8'); const orig = s; let n = 0, skipped = 0;
  for (const x of list) {
    if (s.includes(x.done)) { skipped++; continue; }
    const hits = s.split(x.find).length - 1;
    if (hits !== 1) { console.log(`ANCHOR ${hits === 0 ? 'NOT FOUND' : 'AMBIGUOUS'} in ${file}: ${JSON.stringify(x.find.slice(0, 70))}`); bad++; s = orig; break; }
    s = s.replace(x.find, () => x.repl); n++;
  }
  if (s !== orig) { fs.writeFileSync(p, s); console.log(`patched ${file}: ${n} change(s), ${skipped} already applied`); }
  else console.log(`unchanged ${file}${skipped ? ` (${skipped} already applied)` : ''}`);
}
process.exit(bad ? 1 : 0);
