import {mkdtemp,writeFile,readFile,rm,access} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
const temp=await mkdtemp(path.join(os.tmpdir(),'cbgpu-mcp-package-'));
const npm=(args: string[],cwd: string)=>{
 const result=spawnSync(process.execPath,[process.env.npm_execpath || '',...args],{cwd,encoding:'utf8',timeout:120000});
 assert.equal(result.status,0,result.stderr+'\n'+result.stdout);return result.stdout;
};
let client;
try{
 const [{filename}]=JSON.parse(npm(['pack','--ignore-scripts','--json','--pack-destination',temp],process.cwd()));
 const destination=path.join(temp,'install');await writeFile(path.join(temp,'package.json'),'{}');
 npm(['install',path.join(temp,filename),'--ignore-scripts','--omit=dev','--no-audit','--no-fund','--prefix',destination],temp);
 const root=path.join(destination,'node_modules','cbgpu-deploy-mcp'),pkg=JSON.parse(await readFile(path.join(root,'package.json'),'utf8'));
 assert(!pkg.dependencies||Object.keys(pkg.dependencies).length===0);
 await assert.rejects(()=>access(path.join(destination,'node_modules','@modelcontextprotocol','sdk')));
 const entry=path.join(root,pkg.bin['cbgpu-deploy-mcp']);
 const help=spawnSync(process.execPath,[entry,'--help'],{cwd:temp,encoding:'utf8'});assert.equal(help.status,0,help.stderr);assert(help.stderr.includes('CBGPU Deploy MCP'));assert.equal(help.stdout,'');
 const login=spawnSync(process.execPath,[entry,'login'],{cwd:temp,encoding:'utf8'});assert.equal(login.status,1);assert(login.stderr.includes('interactive terminal'));assert(!login.stderr.includes('ERR_MODULE_NOT_FOUND'));
 const env: NodeJS.ProcessEnv={...process.env,XDG_CONFIG_HOME:path.join(temp,'config'),LOCALAPPDATA:path.join(temp,'config')};delete env.CBGPU_ACCESS_TOKEN;delete env.NODE_PATH;
 client=new Client({name:'isolated-package-test',version:'1'});
 await client.connect(new StdioClientTransport({command:process.execPath,args:[entry],cwd:temp,env:Object.fromEntries(Object.entries(env).filter((entry): entry is [string,string]=>entry[1]!==undefined)),stderr:'pipe'}));
 const {tools}=await client.listTools();assert.equal(tools.length,17);
 const result=await client.callTool({name:'list_projects',arguments:{kind:'frontend'}});assert.equal(result.isError,true);assert(Array.isArray(result.content));assert(result.content[0].type==='text');assert(result.content[0].text.includes('login'));
 console.log('Packed release passed isolated install, CLI startup/login guard and stdio MCP discovery on '+process.version+' '+process.platform+'; no external SDK installed.');
}finally{if(client)await client.close();await rm(temp,{recursive:true,force:true});}
