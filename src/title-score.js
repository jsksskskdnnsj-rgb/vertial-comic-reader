// Title match used when a candidate has no server confidence yet.
// Same shape as the Rust scorer: exact / strong / weak plus 0–100.

const NOISE = ['全年龄', '汉化补丁', '汉化版', '汉化组', '汉化', '漢化', '韩漫', '韩文', '正版', '中文', '简体', '繁体', '官中', '嵌字', '扫图', '裁剪', '无水印', '无水', '完结版', '完结', '连载中', '单行本'];

const HAN = new Map([
  ['國', '国'], ['語', '语'], ['門', '门'], ['書', '书'], ['長', '长'], ['東', '东'], ['車', '车'],
  ['風', '风'], ['龍', '龙'], ['馬', '马'], ['鳥', '鸟'], ['魚', '鱼'], ['頁', '页'], ['見', '见'],
  ['貝', '贝'], ['紅', '红'], ['綠', '绿'], ['藍', '蓝'], ['黃', '黄'], ['學', '学'], ['會', '会'],
  ['體', '体'], ['點', '点'], ['對', '对'], ['發', '发'], ['經', '经'], ['線', '线'], ['網', '网'],
  ['電', '电'], ['話', '话'], ['說', '说'], ['記', '记'], ['愛', '爱'], ['戰', '战'], ['義', '义'],
  ['氣', '气'], ['雲', '云'], ['靈', '灵'], ['劍', '剑'], ['俠', '侠'], ['傳', '传'], ['錄', '录'],
  ['畫', '画'], ['圖', '图'], ['館', '馆'], ['場', '场'], ['後', '后'], ['從', '从'], ['來', '来'],
  ['個', '个'], ['們', '们'], ['這', '这'], ['還', '还'], ['過', '过'], ['進', '进'], ['遠', '远'],
  ['連', '连'], ['運', '运'], ['邊', '边'], ['開', '开'], ['關', '关'], ['問', '问'], ['題', '题'],
  ['無', '无'], ['與', '与'], ['為', '为'], ['時', '时'], ['間', '间'], ['現', '现'], ['實', '实'],
  ['際', '际'], ['機', '机'], ['極', '极'], ['強', '强'], ['難', '难'], ['雜', '杂'], ['雙', '双'],
  ['隻', '只'], ['條', '条'], ['萬', '万'], ['億', '亿'], ['歲', '岁'], ['歷', '历'], ['聖', '圣'],
  ['師', '师'], ['眾', '众'], ['種', '种'], ['裡', '里'], ['麵', '面'], ['擊', '击'], ['鬥', '斗'],
  ['廣', '广'], ['慶', '庆'], ['應', '应'], ['樂', '乐'], ['醫', '医'], ['藥', '药'], ['衛', '卫'],
  ['術', '术'], ['藝', '艺'], ['號', '号'], ['觀', '观'], ['覺', '觉'], ['讓', '让'], ['認', '认'],
  ['識', '识'], ['議', '议'], ['護', '护'], ['變', '变'], ['負', '负'], ['財', '财'], ['貴', '贵'],
  ['費', '费'], ['質', '质'], ['軍', '军'], ['農', '农'], ['達', '达'], ['選', '选'], ['郵', '邮'],
  ['鄉', '乡'], ['針', '针'], ['銀', '银'], ['鏡', '镜'], ['鐵', '铁'], ['隊', '队'], ['階', '阶'],
  ['隨', '随'], ['險', '险'], ['隱', '隐'], ['靜', '静'], ['願', '愿'], ['類', '类'], ['飛', '飞'],
  ['養', '养'], ['餘', '余'], ['龜', '龟'], ['髮', '发'], ['鬱', '郁'], ['節', '节'], ['幹', '干'],
]);

const MACRON = { à: 'a', á: 'a', â: 'a', ä: 'a', ā: 'a', ã: 'a', è: 'e', é: 'e', ê: 'e', ë: 'e', ē: 'e', ì: 'i', í: 'i', î: 'i', ï: 'i', ī: 'i', ò: 'o', ó: 'o', ô: 'o', ö: 'o', ō: 'o', õ: 'o', ù: 'u', ú: 'u', û: 'u', ü: 'u', ū: 'u', ý: 'y', ÿ: 'y', ñ: 'n', ç: 'c' };

export function foldHan(s) {
  let out = '';
  for (const c of String(s ?? '')) out += HAN.get(c) || c;
  return out;
}

export function normaliseTitle(title) {
  let out = '';
  const lower = String(title ?? '').normalize('NFKC').toLowerCase();
  for (const ch of lower) {
    const folded = MACRON[ch] || ch;
    if (/[\p{L}\p{N}]/u.test(folded)) out += folded;
  }
  out = out.replace(/v\d+$/, '');
  for (const noise of NOISE) {
    const n = noise.toLowerCase();
    if (out.endsWith(n)) out = out.slice(0, -n.length);
  }
  return foldHan(out);
}

function dice(a, b) {
  const av = [...a];
  const bv = [...b];
  if (av.length < 2 || bv.length < 2) return 0;
  const left = new Map();
  for (let i = 0; i < av.length - 1; i++) {
    const k = av[i] + av[i + 1];
    left.set(k, (left.get(k) || 0) + 1);
  }
  const right = new Map();
  for (let i = 0; i < bv.length - 1; i++) {
    const k = bv[i] + bv[i + 1];
    right.set(k, (right.get(k) || 0) + 1);
  }
  let inter = 0;
  for (const [k, v] of left) inter += Math.min(v, right.get(k) || 0);
  return (2 * inter) / (av.length - 1 + bv.length - 1);
}

function compareOne(needle, hay) {
  if (!hay) return { kind: 'none', confidence: 0 };
  const n = foldHan(needle);
  const h = foldHan(hay);
  if (needle === hay || (n && n === h)) {
    return { kind: 'exact', confidence: needle === hay ? 100 : 96 };
  }
  const [longer, shorter] = n.length >= h.length ? [n, h] : [h, n];
  if (shorter && longer.startsWith(shorter)) {
    const slack = [...longer].length - [...shorter].length;
    if (slack <= 8 && [...shorter].length >= 3) return { kind: 'strong', confidence: 88 };
    return { kind: 'weak', confidence: 62 };
  }
  if (shorter.length >= 3 && longer.includes(shorter)) return { kind: 'weak', confidence: 58 };
  const d = dice(n, h);
  if (d >= 0.9 && [...n].length >= 4) return { kind: 'strong', confidence: Math.round(78 + d * 18) };
  if (d >= 0.72 && [...n].length >= 4) return { kind: 'weak', confidence: Math.round(48 + d * 30) };
  return { kind: 'none', confidence: Math.round(d * 40) };
}

const RANK = { none: 0, weak: 1, strong: 2, exact: 3 };

/** candidate: { title, originalTitle, aliases } */
export function scoreTitle(query, candidate) {
  const needle = normaliseTitle(query);
  if (!needle) return { kind: 'none', confidence: 0 };
  const names = [candidate?.title, candidate?.originalTitle, ...(candidate?.aliases || [])].filter(Boolean);
  let best = { kind: 'none', confidence: 0 };
  for (const name of names) {
    const cur = compareOne(needle, normaliseTitle(name));
    if (RANK[cur.kind] > RANK[best.kind] || (cur.kind === best.kind && cur.confidence > best.confidence)) best = cur;
  }
  return best;
}
