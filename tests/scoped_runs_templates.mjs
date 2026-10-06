// Real bounded runs/templates acceptance; downstream records are guard fixtures only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { binary, execute, success, fail, write, fingerprint, Fixture, MCP, host, itemPath } from './support/public.mjs';
function definition(f,name,items,edges=[],extra={}) {write(path.join(f.primary,'.work/templates',`${name}.yaml`),{format_version:2,name,items,edges,...extra});}
function rootRun(f) {return success(f.primary,'init').execution.root_run_id;}
function start(f,id) {return success(f.primary,'run','start',id).run.run.id;}
function artifacts(root) {return fingerprint(root).filter(([p])=>/(^|\/)\.(stage|operation|storage)-/.test(p));}
function terminal(f,sid,rid) {write(path.join(f.primary,`.work/scopes/${sid}/runs/${rid}/run.yaml`),{format_version:2,id:rid,created_at:'2026-01-01T00:00:00Z',phase:'abandoned',ended_at:'2026-01-02T00:00:00Z'});}
for(const version of [1,2]) test(`v${version}: material-only selected definitions, symbolic preview, literal parameters and no init`,async()=>{
 const f=new Fixture();let m;try {let before=fingerprint(f.root);assert.deepEqual(success(f.primary,'template','list').templates,[]);assert.deepEqual(fingerprint(f.root),before);m=new MCP(f.primary);await m.init();assert.deepEqual((await m.good('template_list')).templates,[]);assert.deepEqual(fingerprint(f.root),before);
 definition(f,'plan',[{key:'b',title:'{{text}}',body:'{{text}}\nlast',labels:['label={{text}}']},{key:'a',title:'A',model:'item'}],[{from:'local:b',kind:'depends_on',to:'local:a'}],{format_version:version,parameters:['text'],defaults:{model:'template',thinking:'high'}});
 before=fingerprint(f.root);const preview=success(f.primary,'template','preview','plan','--param','text=x={{again}}=z');assert.equal(preview.format_version,version);assert.equal(preview.items[1].title,'x={{again}}=z');assert.equal(preview.items[1].model.source,'default');assert.equal(preview.placement_validated,true);assert.equal(preview.execution.state,'uninitialized');assert.deepEqual(preview,await m.good('template_preview',{name:'plan',parameters:{text:'x={{again}}=z'}}));assert.deepEqual(fingerprint(f.root),before);
 const out=await m.good('template_expand',{name:'plan',parameters:{text:'x={{again}}=z'}});assert.equal(out.items.length,2);assert.equal(out.run_id,null);assert.equal(fs.existsSync(path.join(f.primary,'.work/scopes')),false);assert.equal(fs.existsSync(path.join(f.primary,'.gitignore')),false);const b=success(f.primary,'item','inspect',out.items[1].id).item;assert.deepEqual(b.depends_on,[out.items[0].id]);assert.equal(b.body,'x={{again}}=z\nlast');assert.deepEqual(artifacts(f.root),[]);
 }finally {if(m)await m.close();f.close();}
});
test('standing root parentless wisps and child ancestry are explicit; root/member completion stays independent',async()=>{
 const f=new Fixture();let m;try {const root=rootRun(f);definition(f,'wisp',[{key:'a',title:'Wisp',persistence:'wisp'}]);let out=success(f.primary,'template','expand','wisp');assert.equal(out.run_id,root);const w=out.items[0].id;assert.equal(success(f.primary,'item','inspect',w).item.parent,null);
 const scope=f.create('feature');const child=start(f,scope);const before=fingerprint(f.root);fail(f.primary,'wisp_run_mismatch','template','expand','wisp','--run',child);assert.deepEqual(fingerprint(f.root),before);
 definition(f,'connected',[{key:'a',title:'Material'},{key:'b',title:'Wisp',persistence:'wisp'}],[{from:'local:a',kind:'parent',to:'root'},{from:'local:b',kind:'parent',to:'local:a'}]);out=success(f.primary,'template','expand','connected','--run',child,'--root',scope);let r=success(f.primary,'run','inspect',child).run;assert.ok(r.members.includes(scope));assert.ok(!r.required_members.includes(scope));assert.equal(r.finished,false);
 for(const i of [...out.items].reverse())success(f.primary,'item','close',i.id);r=success(f.primary,'run','inspect',child).run;assert.equal(r.finished,true);assert.equal(success(f.primary,'item','inspect',scope).item.state,'open');m=new MCP(f.primary);await m.init();assert.equal((await m.good('run_start',{scope})).changed,false);assert.equal((await m.good('run_inspect',{run_id:root})).run.finished,null);assert.deepEqual(artifacts(f.root),[]);
 }finally{if(m)await m.close();f.close();}
});
test('start inserts a boundary above stored descendants without inventing item edges',()=>{
 const f=new Fixture();try {rootRun(f);const a=f.create('ancestor'),b=f.create('nested',a);const rb=start(f,b);const ra=start(f,a);const r=success(f.primary,'run','inspect',ra).run;assert.deepEqual(r.descendant_runs,[rb]);assert.equal(success(f.primary,'scope','inspect',b).scope.parent_scope_id,a);assert.equal(success(f.primary,'item','inspect',a).item.parent,null);assert.deepEqual(artifacts(f.root),[]);}finally{f.close();}
});
test('ended barrier, fresh child start and explicit inheritance retain historical wisps',()=>{
 const f=new Fixture();try {rootRun(f);const s=f.create('feature');const old=start(f,s);definition(f,'work',[{key:'w',title:'Old wisp',persistence:'wisp'}],[{from:'local:w',kind:'parent',to:'root'}]);const w=success(f.primary,'template','expand','work','--root',s,'--run',old).items[0].id;terminal(f,s,old);assert.equal(success(f.primary,'item','inspect',s).item.context.execution_state,'ended');let before=fingerprint(f.root);fail(f.primary,'run_not_current','item','reopen',w);assert.deepEqual(fingerprint(f.root),before);const fresh=start(f,s);assert.notEqual(fresh,old);assert.ok(!success(f.primary,'run','inspect',fresh).run.members.includes(w));terminal(f,s,fresh);success(f.primary,'run','inherit',s);assert.equal(success(f.primary,'item','inspect',s).item.context.execution_state,'active');assert.equal(success(f.primary,'item','inspect',w).item.context.execution_state,'ended');}finally{f.close();}
});
test('invalid/physical requests refuse before initialization or catalog publication',async()=>{
 const f=new Fixture();let m;try {definition(f,'demo',[{key:'a',title:'{{text}}'}],[],{parameters:['text']});m=new MCP(f.primary);await m.init();const before=fingerprint(f.root);for(const args of [['template','expand','demo','--param','text=x','--param','text=y'],['template','expand','demo','--param','unknown=x'],['run','start','root'],['template','expand','demo','--param','text=x','--view','physical']]) {const r=execute(f.primary,binary,['--json',...args]);assert.notEqual(r.status,0);assert.equal(JSON.parse(r.stdout).ok,false);}await m.bad('template_expand',{name:'demo',parameters:{text:null}},'invalid_argument');assert.deepEqual(fingerprint(f.root),before);}finally{if(m)await m.close();f.close();}
});

test('human expansion exposes permanent key IDs, authoritative paths and existing updates before init',()=>{
 const f=new Fixture();try{const root=f.create('Root');definition(f,'human',[{key:'material',title:'Material'},{key:'other',title:'Other'}],[{from:'root',kind:'depends_on',to:'local:material'}]);const r=execute(f.primary,binary,['template','expand','human','--root',root]);assert.equal(r.status,0,r.stdout+r.stderr);assert.equal(r.stderr,'');const out=JSON.parse(r.stdout);assert.equal(out.changed,true);assert.equal(out.run_id,null);assert.equal(out.items.length,2);for(const i of out.items){assert.match(i.id,/^[0-9a-f]{32}$/);assert.equal(i.path,itemPath(f.primary,i.id));assert.equal(i.persistence,'material');}assert.deepEqual(out.updated,[{id:root,path:itemPath(f.primary,root)}]);assert.equal(fs.existsSync(path.join(f.primary,'.work/scopes')),false);assert.equal(fs.existsSync(path.join(f.primary,'.gitignore')),false);assert.deepEqual(artifacts(f.root),[]);
 }finally{f.close();}
});
