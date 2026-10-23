import {createRequire} from 'node:module';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
import path from 'node:path';
// 可选外部 Playwright 安装只服务于界面回归，避免运行应用时引入浏览器测试依赖。
// An optional external Playwright installation serves UI regression without adding a browser-test dependency to the running app.
const require=createRequire(process.env.PWC_PLAYWRIGHT_ROOT ? path.join(path.resolve(process.env.PWC_PLAYWRIGHT_ROOT),'package.json') : import.meta.url);
const {chromium}=require('playwright');
const browser=await chromium.launch({headless:true,...(process.env.PWC_BROWSER_EXECUTABLE ? {executablePath:process.env.PWC_BROWSER_EXECUTABLE} : {})});
const artifacts=process.env.PWC_UI_ARTIFACT_DIR || 'target/diagnostics';
await mkdir(artifacts,{recursive:true});
try {
const page=await browser.newPage({viewport:{width:1440,height:960}});
const errors=[];page.on('pageerror',error=>errors.push(error.message));
// 模拟原生桥接以控制乱序、取消和迟到事件，验证前端状态而不写入真实媒体。
// Mock the native bridge to control reordering, cancellation, and delayed events without writing real media.
await page.addInitScript(()=>{
 localStorage.setItem('pwc-language','zh');let n=0;const callbacks=new Map();const listeners=new Map();
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
 const emit=(name,payload)=>{for(const id of listeners.get(name)||[])callbacks.get(id)?.({event:name,id:1,payload});};
 window.__test={emit,requests:[],zooms:[],pending:null};
 window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(callback){callbacks.set(++n,callback);return n;},unregisterCallback(id){callbacks.delete(id);},async invoke(command,args){
  if(command==='plugin:event|listen'){const list=listeners.get(args.event)||[];list.push(args.handler);listeners.set(args.event,list);return n;}
  if(command==='plugin:window|scale_factor')return 1;
  if(command==='plugin:window|is_maximized')return false;
  if(command==='plugin:webview|set_webview_zoom'){window.__test.zooms.push(args.value);return;}
  if(command==='get_launch_context')return {files:[]};
  if(command==='get_release_metadata')return null;
  if(command==='plugin:dialog|open')return Array.from({length:1000},(_,i)=>`/test/photos/${i}.png`);
  if(command==='decode_media'){
   const request=args.request;window.__test.requests.push(request);
   const items=request.inputPaths.map((inputPath,i)=>({inputPath,succeeded:request.recoveryMode==='deep'||i!==0,errorCode:i===0?'recovery_timeout':undefined,error:i===0?'timeout':undefined,mediaType:'image',text:'temporary test',integrity:'intact',correctedCodewords:0,tamperRegions:[],timelineAnomalies:[]}));
   return await new Promise((resolve,reject)=>{window.__test.pending={resolve:()=>resolve({items}),reject,request,items};setTimeout(()=>{for(let i=items.length-1;i>=0;i--)emit('pwc-decode-item',{taskId:request.taskId,current:i+1,total:items.length,item:items[i]});},20);});
  }
  if(command==='cancel_task'){setTimeout(()=>window.__test.pending?.reject('任务已取消'),250);return;}
  return null;
 }};
});
await page.goto(process.env.PWC_UI_URL || 'http://127.0.0.1:1420');
await page.locator('.tabs button').nth(1).click();
await page.locator('.decode-grid .file-picker').first().click();
await page.locator('.decode-grid input[type=password]').fill('temporary test password');
await page.locator('.decode-grid .primary').click();
await page.waitForFunction(()=>document.querySelector('.result-count')?.textContent.includes('1000'));
assert.equal(await page.locator('.decode-item').count(),50);
await page.locator('.tabs button').first().click();
assert.equal(await page.locator('.global-task-bar').count(),1);
assert.ok((await page.locator('.encode-grid .primary').textContent()).includes('开始编码'));
await page.locator('.global-task-bar button').first().click();
assert.ok((await page.locator('.global-task-bar').textContent()).includes('正在取消'));
await page.waitForFunction(()=>document.querySelector('.global-task-bar')?.textContent.includes('已取消'));
await page.evaluate(()=>window.__test.emit('pwc-task-progress',{taskId:window.__test.requests[0].taskId,task:'decode',phase:'geometry-search',message:'late running event',current:1,total:1000,percent:50,outcome:'running'}));
await page.waitForTimeout(50);
assert.ok((await page.locator('.global-task-bar').textContent()).includes('已取消'));
await page.locator('.tabs button').nth(1).click();
assert.equal(await page.locator('.decode-item').count(),50);
await page.locator('.imported-files summary').click();
assert.equal(await page.locator('.imported-file-list li').count(),50);
await page.locator('.imported-file-list button').first().click();
assert.ok((await page.locator('.imported-files summary').textContent()).includes('999'));
await page.locator('.import-summary-head button').click();
assert.equal(await page.locator('.imported-files').count(),0);
await page.locator('.decode-item button').first().click();
await page.waitForFunction(()=>window.__test.requests.length===2);
await page.evaluate(()=>window.__test.pending.resolve());
await page.waitForFunction(()=>document.querySelector('.result-count')?.textContent.includes('成功 1000'));
assert.equal(await page.locator('.decode-item').count(),50);
assert.equal(await page.evaluate(()=>window.__test.requests[1].recoveryMode),'deep');
await page.evaluate(()=>window.__test.emit('pwc-decode-item',{taskId:window.__test.requests[0].taskId,current:1,item:{inputPath:window.__test.requests[0].inputPaths[0],succeeded:false,error:'late failure'}}));
await page.waitForTimeout(50);
assert.ok((await page.locator('.result-count').textContent()).includes('成功 1000'));
await page.screenshot({path:path.join(artifacts,'ui-zh.png')});
await page.getByRole('button',{name:'EN',exact:true}).click();
for(let step=0;step<7;step++)await page.getByRole('button',{name:'Zoom in',exact:true}).click();
await page.waitForFunction(()=>window.__test.zooms.includes(1.35));
// 原生最小尺寸缩放到 135% 后按等效逻辑视口检查，再加一个更窄的压力场景。
// Check the logical viewport equivalent to the native minimum at 135% zoom, followed by a narrower stress case.
for(const viewport of [{width:830,height:563},{width:560,height:380}]){
 await page.setViewportSize(viewport);
 for(const button of await page.locator('.window-controls button').all()){
  const box=await button.boundingBox();assert.ok(box&&box.x>=0&&box.y>=0&&box.x+box.width<=viewport.width&&box.y+box.height<=viewport.height,'window control remains visible');
 }
 assert.equal(await page.locator('.decode-grid').evaluate(el=>getComputedStyle(el).gridTemplateColumns.split(' ').length),1);
 assert.ok(await page.locator('.decode-grid').evaluate(el=>el.scrollHeight>el.clientHeight),'single-column content scrolls rather than shrinking into clipped cards');
 const password=page.locator('.decode-grid input[type=password]');
 await password.scrollIntoViewIfNeeded();
 const inputBox=await password.boundingBox();
 const cardBox=await password.locator('xpath=ancestor::article').boundingBox();
 assert.ok(inputBox&&cardBox&&inputBox.y>=cardBox.y&&inputBox.y+inputBox.height<=cardBox.y+cardBox.height,'credentials remain inside the visible card content');
 await page.locator('.decode-grid').evaluate(el=>{el.scrollTop=0;});
 await page.screenshot({path:path.join(artifacts,viewport.width===830?'ui-en-zoom.png':'ui-en-narrow.png')});
}
assert.deepEqual(errors,[]);
console.log('UI checks passed: 1000 reverse-order results, 50 cards/page, file removal, tab switching, confirmed cancellation, stale events, deep retry after clearing imports, English narrow viewport and window controls');
} finally {
  await browser.close();
}
