// Stable natural ordering for comic page filenames, including full-width and Chinese numerals.
const cnDigits={'零':0,'〇':0,'一':1,'二':2,'两':2,'三':3,'四':4,'五':5,'六':6,'七':7,'八':8,'九':9};
function chineseNumber(s){
 if(/^\d+$/.test(s))return Number(s);
 if(!/[零〇一二两三四五六七八九十百千]/.test(s))return null;
 let total=0,section=0,num=0;for(const c of s){if(c in cnDigits)num=cnDigits[c];else if(c==='十'){section+=(num||1)*10;num=0}else if(c==='百'){section+=(num||1)*100;num=0}else if(c==='千'){section+=(num||1)*1000;num=0}else return null;}return total+section+num;
}
function normalize(s){return String(s??'').normalize('NFKC').toLocaleLowerCase().replace(/[零〇一二两三四五六七八九十百千]+/g,m=>{const n=chineseNumber(m);return n===null?m:String(n).padStart(6,'0')}).replace(/\d+/g,m=>m.padStart(12,'0'));}
export function sortSources(paths){return [...(paths||[])].sort((a,b)=>{const aa=normalize(a),bb=normalize(b);return aa.localeCompare(bb,undefined,{numeric:true,sensitivity:'base'})||String(a).localeCompare(String(b));});}
