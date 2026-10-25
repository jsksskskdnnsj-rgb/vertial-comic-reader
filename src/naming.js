// Stable natural ordering for comic filenames, including full-width and Chinese numerals.
// v0.6: compareNames / sortBy added so books can be sorted as objects (the v0.5.1 library sorted
// names and looked the books back up by name, which duplicated one of two same-named books and
// dropped the other). sortSources is unchanged.
// v0.6.2: Chinese numerals are converted ONLY in chapter-style names (第N话/章/卷/集/回/部/期/节/篇), no longer
// anywhere in a name: 《二十四孝》 and 《十万个冷笑话》 used to be ordered by the numbers 24 and 10, and any title
// starting with 一/二/三/十 was mangled. New: compareTitles / sortByTitle, an alphabetical A-Z order with zh-CN
// collation (Chinese by reading) for the library's "Title A-Z" sort; it never converts numerals.
const cnDigits={'零':0,'〇':0,'一':1,'二':2,'两':2,'三':3,'四':4,'五':5,'六':6,'七':7,'八':8,'九':9};
function chineseNumber(s){
 if(/^\d+$/.test(s))return Number(s);
 if(!/[零〇一二两三四五六七八九十百千]/.test(s))return null;
 let total=0,section=0,num=0;for(const c of s){if(c in cnDigits)num=cnDigits[c];else if(c==='十'){section+=(num||1)*10;num=0}else if(c==='百'){section+=(num||1)*100;num=0}else if(c==='千'){section+=(num||1)*1000;num=0}else return null;}return total+section+num;
}
// Chinese numerals count only right after 第 and before a counter word: 第十一话 -> 第11话. Anything else is plain text.
const CN_COUNTED=/第([零〇一二两三四五六七八九十百千]+)(?=[话話章卷集回部期节節篇])/g;
export function numeralsToDigits(s){return String(s??'').normalize('NFKC').replace(CN_COUNTED,(m,n)=>{const v=chineseNumber(n);return v===null?m:'第'+v});}
function normalize(s){return numeralsToDigits(s).toLocaleLowerCase().replace(/\d+/g,m=>m.padStart(12,'0'));}
export function compareNames(a,b){return normalize(a).localeCompare(normalize(b),undefined,{numeric:true,sensitivity:'base'})||String(a).localeCompare(String(b));}
export function sortSources(paths){return [...(paths||[])].sort(compareNames);}
export function sortBy(list,get){return [...(list||[])].sort((a,b)=>compareNames(get(a),get(b)));}

// ---- Title A-Z (library sort) ------------------------------------------------------------------------------
// Intl.Collator('zh-CN'): Latin before Han, Han by pinyin reading, case-insensitive, digit runs by value ("Vol 2" < "Vol 10").
// Leading punctuation, brackets and spaces are ignored so 《一人之下》 sorts under 一, not under 《.
let COLLATOR;
try { COLLATOR=new Intl.Collator('zh-CN',{numeric:true,sensitivity:'base'}); } catch { COLLATOR=new Intl.Collator(undefined,{numeric:true,sensitivity:'base'}); }
const titleKey=(s)=>{const t=String(s??'').normalize('NFKC').trim();return t.replace(/^[\p{P}\p{S}\s]+/u,'')||t;};
export function compareTitles(a,b){const x=titleKey(a),y=titleKey(b);return COLLATOR.compare(x,y)||(String(a)<String(b)?-1:String(a)>String(b)?1:0);}
export function sortByTitle(list,get=(x)=>x){return [...(list||[])].sort((a,b)=>compareTitles(get(a),get(b)));}
