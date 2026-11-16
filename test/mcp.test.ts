import type {TestContext} from 'node:test';
import type {DeploymentClient,Fetcher} from '../src/api.ts';
import {payload as readPayload,toolText,type Call} from './helpers.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,stat,chmod,writeFile} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {DeployAPI,baseURL,sanitize} from '../src/api.ts';
import {createServer} from '../src/server.ts';
const project='550e8400-e29b-41d4-a716-446655440000';
const domain='550e8400-e29b-41d4-a716-446655440001';
const json=(data: unknown,status=200)=>new Response(JSON.stringify(data),{status});
async function fixture(t: TestContext,handler: Fetcher){const dir=await mkdtemp(path.join(os.tmpdir(),'cbgpu-mcp-'));t.after(()=>rm(dir,{recursive:true,force:true}));return new DeployAPI({file:path.join(dir,'auth','session.json'),fetcher:handler,token:undefined});}
async function clientFor(t: TestContext,api: DeploymentClient){const server=createServer(api),client=new Client({name:'test',version:'1'});const [a,b]=InMemoryTransport.createLinkedPair();await Promise.all([server.connect(a),client.connect(b)]);t.after(()=>Promise.all([client.close(),server.close()]));return client;}
test('MCP discovery, inputs and tools execute through real SDK transport',async t=>{
 const calls: Call[]=[];const client=await clientFor(t,{request:async(...args: Call)=>{calls.push(args);return{projects:[],env_vars:{SECRET:'hidden'}};}});
 const tools=(await client.listTools()).tools;
 assert.equal(tools.length,17);assert.equal(tools.find(t=>t.name==='list_projects')?.annotations?.readOnlyHint,true);
 assert(!tools.some(t=>/shell|sql|delete|password/.test(t.name)));
 const response=await client.callTool({name:'list_projects',arguments:{kind:'frontend'}});
 assert.deepEqual(calls[0],['/projects']);assert(!toolText(response).includes('hidden'));
 const bad=await client.callTool({name:'create_frontend',arguments:{slug:'valid-name',repository:'https://github.com/x/y',root_directory:'../private'}});assert.equal(bad.isError,true);assert.equal(calls.length,1);
});
test('server update preserves environment and maps subfolder/runtime',async t=>{
 const calls: Call[]=[];const client=await clientFor(t,{request:async(...args: Call)=>{calls.push(args);return args[1]?{ok:true}:{project:{env_vars:{PASSWORD:'do-not-return',CBGPU_DEPLOY_ENGINE:'compose'},collaboration_role:'owner'}};}});
 await client.callTool({name:'update_server',arguments:{project_id:project,settings:{root_directory:'server',memory_limit:'16g'}}});
 assert.deepEqual(calls[1],['/servers/projects/'+project,'PATCH',{memory_limit:'16g',env_vars:{PASSWORD:'do-not-return',CBGPU_DEPLOY_ENGINE:'compose',CBGPU_ROOT_DIRECTORY:'server'}}]);
});
test('Compose creation and frontend redeploy use correct asynchronous APIs',async t=>{
 const calls: Call[]=[];const client=await clientFor(t,{request:async(...args: Call)=>{calls.push(args);return{deployment_id:project};}});
 const result=await client.callTool({name:'create_server',arguments:{name:'My backend',mode:'compose',settings:{git_url:'https://github.com/example/app',root_directory:'server',container_port:8000,gateway_service:'api-gw',cpu_limit:3,memory_limit:'4g'}}});
 assert(!result.isError);const payload=readPayload(calls[0]);assert.equal(payload.env_vars.CBGPU_DEPLOY_ENGINE,'compose');assert.equal(payload.env_vars.CBGPU_ROOT_DIRECTORY,'server');assert.equal(payload.env_vars.CBGPU_STORAGE_GB,'40');assert.equal(payload.deploy_type,'git');assert.equal(payload.run_command,'docker compose up -d');
 await client.callTool({name:'redeploy',arguments:{kind:'frontend',project_id:project}});assert.deepEqual(calls[1],['/projects/'+project+'/deploy','POST',{}]);
 await client.callTool({name:'redeploy',arguments:{kind:'server',project_id:project}});assert.equal(calls[2][0],'/servers/projects/'+project);assert(readPayload(calls[2]).deploy_requested_at);
 await client.callTool({name:'verify_domain',arguments:{kind:'server',project_id:project,domain_id:domain}});assert.deepEqual(readPayload(calls[3]),{domain_id:domain});
});
test('Compose health failures stay failures; known environment secrets are removed from logs',async t=>{
 const client2=await clientFor(t,{request:async (endpoint: string)=>endpoint.startsWith('/servers/')?{project:{env_vars:{PASSWORD:'super-secret',CBGPU_DEPLOY_ENGINE:'compose'}}}:{status:'error',logs:'DB error super-secret',error:'rest unhealthy'}});
 const result=await client2.callTool({name:'deployment_logs',arguments:{kind:'server',project_id:project}});
 assert(!result.isError);assert(!toolText(result).includes('super-secret'));assert(toolText(result).includes('rest unhealthy'));assert(toolText(result).includes('"error"'));
});
test('authorization denial produces a tool error and no mutation',async t=>{
 const calls: Call[]=[];const client=await clientFor(t,{request:async(...args: Call)=>{calls.push(args);return{project:{env_vars:{},collaboration_role:'viewer'}};}});
 const result=await client.callTool({name:'update_server',arguments:{project_id:project,settings:{name:'Change'}}});assert.equal(result.isError,true);assert.equal(calls.length,1);
});
test('login stores private session, never password, refresh is serialized across instances',async t=>{
 let refresh=0;const api=await fixture(t,async(url,options)=>{
  assert.equal(options.redirect,'error');assert.equal(options.headers.Origin,'https://cbgpu.com');
  if(url.includes('grant_type=password'))return json({access_token:'old',refresh_token:'refresh',expires_at:1,user:{email:'test@example.com'}});
  if(url.includes('grant_type=refresh_token')){refresh++;await new Promise(r=>setTimeout(r,50));return json({access_token:'new',refresh_token:'new-refresh',expires_in:3600});}
  assert(options.headers.Authorization);return json({projects:[]});
 });
 await api.login('test@example.com','never-store-this');
 if(process.platform!=='win32')assert.equal((await stat(api.file)).mode & 0o777,0o600);const session=await api.read();assert(!JSON.stringify(session).includes('never-store-this'));
 const other=new DeployAPI({file:api.file,fetcher:api.fetcher});await Promise.all([api.request('/projects'),other.request('/servers/projects')]);assert.equal(refresh,1);
});
test('401 refreshes once; 403 and ambiguous network failure never retry a deployment',async t=>{
 let calls=0,refresh=0;const api=await fixture(t,async(url,options)=>{
  if(url.includes('refresh_token')){refresh++;return json({access_token:'new',refresh_token:'refresh2',expires_in:3600});}
  calls++;return options.headers.Authorization==='Bearer old'?json({},401):json({ok:true});
 });
 await api.prepare();await api.save({access_token:'old',refresh_token:'r',expires_at:Date.now()/1000+3600});
 await api.request('/projects/'+project+'/deploy','POST',{});assert.equal(refresh,1);assert.equal(calls,2);
 api.fetcher=async()=>{throw Error('network gone');};await assert.rejects(()=>api.request('/projects/'+project+'/deploy','POST',{}),/network gone/);
 api.fetcher=async()=>json({},403);await assert.rejects(()=>api.request('/projects'),/permission/);assert.equal(refresh,1);
});
test('expired logout revokes only this session; deletion follows successful revoke',async t=>{
 const urls: string[]=[];const api=await fixture(t,async url=>{urls.push(url);return url.includes('refresh_token')?json({access_token:'fresh'}):new Response(null,{status:204});});
 await api.prepare();await api.save({access_token:'old',refresh_token:'r',expires_at:1});await api.logout();assert(urls[1].endsWith('/logout?scope=local'));await assert.rejects(()=>stat(api.file),{code:'ENOENT'});
});
test('restricts destination, endpoints, credential files and output secrets',async t=>{
 for(const value of ['http://cbgpu.com','https://user:password@cbgpu.com','https://cbgpu.com/x','https://cbgpu.com/?x=1'])assert.throws(()=>baseURL(value));
 const api=await fixture(t,()=>{throw Error('should not fetch');});await assert.rejects(()=>api.request('/../admin'),/Unsupported/);
 await api.prepare();await api.save({access_token:'secret',refresh_token:'synthetic',expires_at:1,base:'https://other.com'});if(process.platform!=='win32'){await chmod(api.file,0o644);await assert.rejects(()=>api.read(),/private/);await chmod(api.file,0o600);}
 await writeFile(api.file,JSON.stringify({base:'https://other.com'}));await assert.rejects(()=>api.read(),/different/);
 const result=sanitize({env_vars:{FOO:'secret'},access_token:'secret',repository:'https://name:secret@github.com/x/y',logs:'ghp_abcdef123'});assert(!JSON.stringify(result).includes('secret'));assert(result&&typeof result==='object'&&'environment_keys' in result);assert.deepEqual(result.environment_keys,['FOO']);
});
