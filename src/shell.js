import zh from './zh-CN.js';
import en from './en.js';
export const T=window.__TAURI__||{};
export const invoke=(cmd,args={})=>{if(!T.core?.invoke)return Promise.reject(new Error('桌面端接口不可用'));return T.core.invoke(cmd,args)};
export const win=T.window?.getCurrentWindow?.()||{setFullscreen:async()=>{},isFullscreen:async()=>false,onCloseRequested:async()=>{},destroy:async()=>{}};
const lang=()=>localStorage.getItem('mh.lang')||'zh-CN';
export function t(key,vars={}){const dict=lang()==='en'?en:zh;let s=dict[key]??en[key]??zh[key]??key;return String(s).replace(/\{(\w+)\}/g,(_,k)=>vars[k]??`{${k}}`).replace(/\\u([\da-f]{4})/gi,(_,h)=>String.fromCharCode(parseInt(h,16)));}
export const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function ago(v){if(!v)return '';let n=typeof v==='number'?(v<1e12?v*1000:v):Date.parse(v);if(!Number.isFinite(n))return '';const en=lang()==='en';const d=Math.max(0,Date.now()-n),m=Math.floor(d/60000);if(m<1)return en?'just now':'刚刚';if(m<60)return en?`${m}m ago`:`${m} 分钟前`;const h=Math.floor(m/60);if(h<24)return en?`${h}h ago`:`${h} 小时前`;const days=Math.floor(h/24);return days<30?(en?`${days}d ago`:`${days} 天前`):new Date(n).toLocaleDateString();}
export const prefs={get(k,d){try{const v=localStorage.getItem('mh.pref.'+k);return v===null?d:JSON.parse(v)}catch{return d}},set(k,v){try{localStorage.setItem('mh.pref.'+k,JSON.stringify(v))}catch{}}};
export function toast(msg){let n=document.querySelector('.toast');if(!n){n=document.createElement('div');n.className='toast';document.body.append(n)}n.textContent=String(msg??'');n.classList.add('show');clearTimeout(n._timer);n._timer=setTimeout(()=>n.classList.remove('show'),2600)}
export function bootPage(section){document.documentElement.lang=lang()==='en'?'en':'zh-CN';document.querySelectorAll('[data-i]').forEach(e=>e.textContent=t(e.dataset.i));document.querySelectorAll('[data-i-label]').forEach(e=>e.setAttribute('aria-label',t(e.dataset.iLabel)));document.title=t('app.name');}
export const isModalOpen=()=>!!document.querySelector('.modal-backdrop');
export function openModal({title='',body=null,wide=false}={}){const backdrop=document.createElement('div');backdrop.className='modal-backdrop';const panel=document.createElement('section');panel.className='modal glass-strong'+(wide?' modal--wide':'');const head=document.createElement('header');head.className='modal__head';const h=document.createElement('h2');h.textContent=title;const close=document.createElement('button');close.className='ibtn';close.textContent='×';close.setAttribute('aria-label','Close');head.append(h,close);const content=document.createElement('div');content.className='modal__body';if(typeof body==='string')content.innerHTML=body;else if(body)content.append(body);panel.append(head,content);backdrop.append(panel);document.body.append(backdrop);const done=()=>backdrop.remove();close.onclick=done;backdrop.addEventListener('click',e=>{if(e.target===backdrop)done()});return {close:done,element:panel,body:content}}
function dialog({title='',message='',ok='确定',cancel='取消',danger=false,placeholder=''}){return new Promise(resolve=>{const body=document.createElement('div');body.className='dialog';const p=document.createElement('p');p.textContent=message;body.append(p);let input;if(placeholder){input=document.createElement('input');input.className='field';input.placeholder=placeholder;body.append(input)}const actions=document.createElement('div');actions.className='modal__actions';const no=document.createElement('button');no.className='btn btn--ghost';no.textContent=cancel;const yes=document.createElement('button');yes.className='btn '+(danger?'btn--danger':'btn--primary');yes.textContent=ok;actions.append(no,yes);body.append(actions);const m=openModal({title,body});no.onclick=()=>{m.close();resolve(false)};yes.onclick=()=>{const v=input?input.value:true;m.close();resolve(v)};if(input)input.focus()})}
export const confirmDialog=o=>dialog(o);
export async function choiceDialog({title,message='',choices=[]}={}){return new Promise(resolve=>{const body=document.createElement('div');body.className='dialog';const p=document.createElement('p');p.textContent=message;body.append(p);const actions=document.createElement('div');actions.className='modal__actions';const m=openModal({title,body});for(const c of choices){const b=document.createElement('button');b.className='btn '+(c.kind==='primary'?'btn--primary':'btn--ghost');b.textContent=c.label;b.onclick=()=>{m.close();resolve(c.value)};actions.append(b)}body.append(actions)})}
export const promptDialog=o=>dialog({...o,placeholder:o?.placeholder||''});


// Compatibility exports for the surrounding app shell. They are intentionally idempotent.
export { lang };
export function applyI18n(root=document){
  root.querySelectorAll('[data-i]').forEach(e=>{e.textContent=t(e.dataset.i)});
  root.querySelectorAll('[data-i-label]').forEach(e=>e.setAttribute('aria-label',t(e.dataset.iLabel)));
  root.querySelectorAll('[data-i-title]').forEach(e=>e.setAttribute('title',t(e.dataset.iTitle)));
  document.documentElement.lang=lang()==='en'?'en':'zh-CN';
}
export function applyTheme(theme=prefs.get('theme','dark')){
  const safe=['dark','light','system'].includes(theme)?theme:'dark';
  document.documentElement.dataset.theme=safe;
  document.documentElement.classList.toggle('theme-light',safe==='light');
  return safe;
}
export function wireWindow(){
  const w=T.window?.getCurrentWindow?.(); if(!w)return false;
  document.querySelectorAll('[data-window-minimize]').forEach(b=>b.addEventListener('click',()=>w.minimize?.()));
  document.querySelectorAll('[data-window-maximize]').forEach(b=>b.addEventListener('click',()=>w.toggleMaximize?.()));
  document.querySelectorAll('[data-window-close]').forEach(b=>b.addEventListener('click',()=>w.close?.()));
  return true;
}
