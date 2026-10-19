const base='http://127.0.0.1:9229';
let page:any;
for(let i=0;i<160;i++){
  try{ page=(await (await fetch(base+'/json/list')).json()).find((t:any)=>t.type==='page'); if(page)break;}catch{}
  await Bun.sleep(50);
}
if(!page)throw Error('CEF source target unavailable');
const ws=new WebSocket(page.webSocketDebuggerUrl);
await new Promise<void>((resolve,reject)=>{ws.onopen=()=>resolve();ws.onerror=()=>reject(Error('CDP connection failed'));});
let next=0;const pending=new Map<number,(v:any)=>void>();
ws.onmessage=e=>{let v=JSON.parse(String(e.data));if(v.id){pending.get(v.id)?.(v);pending.delete(v.id);}};
const rpc=(method:string,params:any={})=>new Promise<any>(resolve=>{let id=++next;pending.set(id,resolve);ws.send(JSON.stringify({id,method,params}));});
const media=async()=>{let r=await rpc('Runtime.evaluate',{expression:'({dark:matchMedia("(prefers-color-scheme: dark)").matches,light:matchMedia("(prefers-color-scheme: light)").matches})',returnByValue:true});return r.result.result.value;};
const change=async(theme:string)=>{
 const command=Bun.spawn(['xfconf-query','-c','xsettings','-p','/Net/ThemeName','-s',theme],{stdout:'pipe',stderr:'pipe'});
 if(await command.exited)throw Error(await new Response(command.stderr).text());
 const expected=theme==='Arc-Dark';
 let value:any;
 for(let i=0;i<100;i++){value=await media();if(value.dark===expected)break;await Bun.sleep(30);}
 console.log(JSON.stringify({osTheme:theme,sourceMedia:value}));
 await Bun.sleep(500);
 if(value.dark!==expected)throw Error('Source media failed to track OS theme');
};
console.log(JSON.stringify({initial:await media()}));
try{await change('Arc-Dark');await change('Arc');await change('Arc-Dark');}
finally{const p=Bun.spawn(['xfconf-query','-c','xsettings','-p','/Net/ThemeName','-s','Xfce']);await p.exited;console.log(JSON.stringify({restored:await media()}));ws.close();}
