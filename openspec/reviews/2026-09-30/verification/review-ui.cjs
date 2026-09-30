// Diagnostic reproduction: a passing assertion confirms the pre-fix defect.
// Runs the actual main.js state handlers with mocked Tauri/DOM/timers; no browser or app starts.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '../../../..');
const elements = new Map();
function element() {
  const classes = new Set(['hidden']);
  return {value:'', style:{}, listeners:{},
    classList:{add:(...xs)=>xs.forEach(x=>classes.add(x)),remove:(...xs)=>xs.forEach(x=>classes.delete(x)),contains:x=>classes.has(x)},
    addEventListener(name, fn){this.listeners[name]=fn;}, appendChild(){}};
}
const events = new Map(), timers = new Map();
let nextId = 0;
const context = {
  console,
  window:{__TAURI__:{
    core:{invoke:async name=>name==='get_config'?{}:[]},
    dialog:{open:async()=>null},
    event:{listen:(name, fn)=>{events.set(name,fn);return Promise.resolve(()=>{});}}
  }},
  document:{querySelector:s=>{if(!elements.has(s))elements.set(s,element());return elements.get(s);},
    createElement:element,createTextNode:x=>x},
  setTimeout:(fn)=>{timers.set(++nextId,fn);return nextId;},
  clearTimeout:id=>timers.delete(id),
};
vm.runInNewContext(fs.readFileSync(path.join(root,'src/main.js'),'utf8'),context);
const progress = events.get('import-progress');
progress({payload:{file_name:'first.zip',percent:100,status:'완료'}});
progress({payload:{file_name:'second.zip',percent:10,status:'가져오는 중...'}});
assert.equal(elements.get('#progress-section').classList.contains('hidden'),false);
for (const callback of timers.values()) callback();
assert.equal(elements.get('#progress-section').classList.contains('hidden'),true);
console.log('R16 reproduced: a previous job timer hides the next active job.');
const permissions=JSON.parse(fs.readFileSync(path.join(root,'src-tauri/capabilities/default.json'),'utf8')).permissions;
const manifests=JSON.parse(fs.readFileSync(path.join(root,'src-tauri/gen/schemas/acl-manifests.json'),'utf8'));
const defaults=manifests['core:window'].default_permission.permissions;
assert(!permissions.includes('core:window:allow-close'));
assert(!defaults.includes('allow-close'));
console.log('R17 verified from generated ACL: window.close is not permitted.');
