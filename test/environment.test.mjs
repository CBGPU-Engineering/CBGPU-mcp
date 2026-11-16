import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm,symlink} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {environmentFile,environmentChanges,setEnvironment,listEnvironment,environmentCommand} from '../src/environment.mjs';
import {createServer} from '../src/server.mjs';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
const id='550e8400-e29b-41d4-a716-446655440000';
async function fixture(t){const dir=await mkdtemp(path.join(tmpdir(),'cbgpu-env-'));t.after(()=>rm(dir,{recursive:true,force:true}));return dir;}
test('dotenv handles quotes, multiline, comments, empty values and literal expansion without shell execution',async t=>{
 const dir=await fixture(t),file=path.join(dir,'.env.production');
 await writeFile(file,'TOKEN="secret # value"\nMULTI="first\\nsecond"\nEMPTY=\nLITERAL=${DO_NOT_READ}\nCOMMAND=$(do-not-execute)\n');
 const values=await environmentFile(file);assert.equal(values.TOKEN,'secret # value');assert.equal(values.MULTI,'first\nsecond');assert.equal(values.EMPTY,'');assert.equal(values.LITERAL,'${DO_NOT_READ}');assert.equal(values.COMMAND,'$(do-not-execute)');
 assert.throws(()=>environmentChanges('server',{CBGPU_DEPLOY_ENGINE:'git'}),/reserved/);assert.throws(()=>environmentChanges('frontend',{SECRET:'a'.repeat(8193)}));
 await writeFile(file,'a'.repeat(131073));await assert.rejects(()=>environmentFile(file),/Cannot read/);
 await assert.rejects(()=>environmentFile(path.join(dir,'credentials.json')),/absolute path/);
 await writeFile(file,'TOKEN=secret');const link=path.join(dir,'.env.link');
 try{await symlink(file,link);await assert.rejects(()=>environmentFile(link),/Cannot read/);}catch(error){if(process.platform!=='win32'||!['EPERM','EACCES'].includes(error.code))throw error;}
});
test('server changes preserve runtime configuration and unrelated secrets, return names only and never deploy',async()=>{
 const calls=[];const api={request:async(...args)=>{calls.push(args);return{project:{collaboration_role:'editor',env_vars:{CBGPU_DEPLOY_ENGINE:'compose',OLD:'old-private',REMOVE:'private'}}};}};
 const result=await setEnvironment(api,'server',id,{TOKEN:'new-private',REMOVE:null});
 assert.deepEqual(calls[1],['/servers/projects/'+id,'PATCH',{env_vars:{CBGPU_DEPLOY_ENGINE:'compose',OLD:'old-private',TOKEN:'new-private'}}]);
 assert.deepEqual(result.updated_keys,['TOKEN']);assert.deepEqual(result.removed_keys,['REMOVE']);assert.equal(result.redeploy_required,true);assert(!JSON.stringify(result).includes('private'));assert.equal(calls.length,2);
});
test('frontend sends only changed variables; viewers cannot edit; list never returns values',async()=>{
 const calls=[];const api={request:async(...args)=>{calls.push(args);return{project:{collaboration_role:'owner',environment_keys:['OLD','REMOVE'],env_vars:{KEY:'private',CBGPU_ROOT_DIRECTORY:'server'}}};}};
 await setEnvironment(api,'frontend',id,{TOKEN:'secret',REMOVE:null});assert.deepEqual(calls[1][2],{env:{TOKEN:'secret',REMOVE:null}});
 const result=await listEnvironment(api,'server',id);assert.deepEqual(result.environment_keys,['KEY','OLD','REMOVE']);assert(!JSON.stringify(result).includes('private'));
 let writes=0;const viewer={request:async(_p,m)=>{if(m)writes++;return{project:{collaboration_role:'viewer'}};}};await assert.rejects(()=>setEnvironment(viewer,'server',id,{KEY:'secret'}),/owner or editor/);assert.equal(writes,0);
});
test('MCP env file tool and CLI import keep file contents out of responses',async t=>{
 const dir=await fixture(t),file=path.join(dir,'.env');await writeFile(file,'API_KEY=do-not-expose-this\nPORT=8080\n');
 const calls=[];const api={request:async(...args)=>{calls.push(args);return{project:{collaboration_role:'owner',env_vars:{}}};}};
 const server=createServer(api),client=new Client({name:'env-test',version:'1'}),[a,b]=InMemoryTransport.createLinkedPair();await Promise.all([server.connect(a),client.connect(b)]);t.after(()=>Promise.all([client.close(),server.close()]));
 const result=await client.callTool({name:'set_environment_from_file',arguments:{kind:'server',project_id:id,env_file:file}});assert(!result.isError);assert(result.content[0].text.includes('API_KEY'));assert(!JSON.stringify(result).includes('do-not-expose-this'));assert.equal(calls[1][2].env_vars.API_KEY,'do-not-expose-this');
 const cli=await environmentCommand(api,['set','server',id,'--file',file]);assert.equal(cli.ok,true);assert(!JSON.stringify(cli).includes('do-not-expose-this'));
 await environmentCommand(api,['unset','server',id,'--keys','API_KEY']);assert.equal(calls.at(-1)[1],'PATCH');
});
