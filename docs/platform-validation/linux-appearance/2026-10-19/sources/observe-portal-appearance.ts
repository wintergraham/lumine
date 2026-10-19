let page:any;for(let i=0;i<160;i++){try{page=(await (await fetch('http://127.0.0.1:9230/json/list')).json()).find((t:any)=>t.type==='page');if(page)break;}catch{}await Bun.sleep(50);}
if(!page)throw Error('Normal Chromium source target unavailable');
const ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise<void>((resolve,reject)=>{ws.onopen=()=>resolve();ws.onerror=()=>reject(Error('CDP connection'));});
let next=0;const pending=new Map<number,(v:any)=>void>();ws.onmessage=e=>{let v=JSON.parse(String(e.data));if(v.id){pending.get(v.id)?.(v);pending.delete(v.id);}};
const rpc=(method:string,params:any={})=>new Promise<any>(resolve=>{let id=++next;pending.set(id,resolve);ws.send(JSON.stringify({id,method,params}));});
const media=async()=>{let r=await rpc('Runtime.evaluate',{expression:'({dark:matchMedia("(prefers-color-scheme: dark)").matches,light:matchMedia("(prefers-color-scheme: light)").matches})',returnByValue:true});return r.result.result.value;};
const portal=async()=>{let p=Bun.spawn(['gdbus','call','--session','--dest','org.freedesktop.portal.Desktop','--object-path','/org/freedesktop/portal/desktop','--method','org.freedesktop.portal.Settings.ReadOne','org.freedesktop.appearance','color-scheme'],{stdout:'pipe'});let result=await new Response(p.stdout).text();if(await p.exited)throw Error('Portal read');return result.trim();};
console.log(JSON.stringify({browser:(await rpc('Browser.getVersion')).result,initial:await media(),portal:await portal(),gtkTheme:'Arc-Dark'}));
for(const setting of ['prefer-light','prefer-dark','default']){
 const started=performance.now();let p=Bun.spawn(['gsettings','set','org.gnome.desktop.interface','color-scheme',setting],{stdout:'pipe',stderr:'pipe'});if(await p.exited)throw Error(await new Response(p.stderr).text());
 const expectedDark=setting!=='prefer-light';let value:any;for(let i=0;i<100;i++){value=await media();if(value.dark===expectedDark)break;await Bun.sleep(50);}
 console.log(JSON.stringify({gsettings:setting,sourceMedia:value,portal:await portal(),elapsedMs:performance.now()-started}));
 if(value.dark!==expectedDark)throw Error('Real portal preference did not reach source CSS');
 await Bun.sleep(500);
}
ws.close();
