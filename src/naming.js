// Stable natural ordering for comic filenames, including full-width and Chinese numerals.
// v0.6: compareNames / sortBy added so books can be sorted as objects (the v0.5.1 library sorted
// names and looked the books back up by name, which duplicated one of two same-named books and
// dropped the other). sortSources is unchanged.
const cnDigits={'零':0,'〇':0,'一':1,'二':2,'两':2,'三':3,'四':4,'五':5,'六':6,'七':7,'八':8,'九':9};
function chineseNumber(s){
 if(/^\d+$/.test(s))return Number(s);
 if(!/[零〇一二两三四五六七八九十百千]/.test(s))return null;
 let total=0,section=0,num=0;for(const c of s){if(c in cnDigits)num=cnDigits[c];else if(c==='十'){section+=(num||1)*10;num=0}else if(c==='百'){section+=(num||1)*100;num=0}else if(c==='千'){section+=(num||1)*1000;num=0}else return null;}return total+section+num;
}
function normalize(s){return String(s??'').normalize('NFKC').toLocaleLowerCase().replace(/[零〇一二两三四五六七八九十百千]+/g,m=>{const n=chineseNumber(m);return n===null?m:String(n).padStart(6,'0')}).replace(/\d+/g,m=>m.padStart(12,'0'));}
export function compareNames(a,b){return normalize(a).localeCompare(normalize(b),undefined,{numeric:true,sensitivity:'base'})||String(a).localeCompare(String(b));}
export function sortSources(paths){return [...(paths||[])].sort(compareNames);}
export function sortBy(list,get){return [...(list||[])].sort((a,b)=>compareNames(get(a),get(b)));}
