import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
const ROOT = fileURLToPath(new URL('../dist/', import.meta.url));
const MIME={'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'};
const server=http.createServer(async(req,res)=>{try{const p=normalize(decodeURIComponent(req.url.split('?')[0]));const f=join(ROOT,p==='/'?'index.html':p);const b=await readFile(f);res.writeHead(200,{'content-type':MIME[extname(f)]??'application/octet-stream'});res.end(b);}catch{res.writeHead(404);res.end('nf')}});
await new Promise(r=>server.listen(4605,'127.0.0.1',r));
const browser=await chromium.launch();
const page=await browser.newPage({viewport:{width:1360,height:880}});
page.on('pageerror', e=>console.log('PAGEERROR', String(e)));
await page.goto('http://127.0.0.1:4605/',{waitUntil:'networkidle'});
await page.waitForTimeout(800);
await page.getByRole('button',{name:'+ 新建组件'}).click();
await page.waitForTimeout(400);
console.log(await page.evaluate(() => {
  const modal = document.querySelector('.modal');
  const checks = modal.querySelectorAll('.check').length;
  const deps = modal.querySelectorAll('.flex.wrap .check').length;
  const steprows = modal.querySelectorAll('.steprow').length;
  const btn = modal.querySelector('.row .btn');
  return { checks, deps, steprows, btnWidth: btn ? Math.round(btn.getBoundingClientRect().width) : -1,
           btnText: btn?.textContent?.trim() };
}));
await browser.close(); server.close();
