#!/usr/bin/env node
import {createInterface} from 'node:readline/promises';
import {Writable} from 'node:stream';
import {StdioServerTransport} from '@modelcontextprotocol/sdk/server/stdio.js';
import {DeployAPI,sessionPath} from './api.mjs';
import {createServer} from './server.mjs';
const api = new DeployAPI();
async function login() {
 if (!process.stdin.isTTY || !process.stderr.isTTY) throw Error('Run login in an interactive terminal. Never supply your password in a prompt, command argument or MCP configuration.');
 let muted = false;
 const output = new Writable({write(chunk,encoding,done){if(!muted)process.stderr.write(chunk,encoding);done();}});
 const input = createInterface({input:process.stdin,output,terminal:true});
 try {
  const email = (await input.question('CBGPU email: ')).trim();
  process.stderr.write('CBGPU password (hidden): '); muted=true;
  const password = await input.question(''); muted=false;process.stderr.write('\n');
  const account=await api.login(email,password);
  console.error('Connected as '+account.email+'. Credentials saved privately at '+sessionPath()+'. Your password was not saved.');
 } finally {muted=false;input.close();}
}
try {
 const command = process.argv[2] || 'serve';
 if(command==='login') await login();
 else if(command==='logout'){await api.logout();console.error('CBGPU MCP session revoked and removed.');}
 else if(command==='status'){const data=await api.request('/projects');console.error('Authenticated. Accessible frontend projects: '+data.projects.length);}
 else if(command==='--help'||command==='help')console.error('CBGPU Deploy MCP\n  cbgpu-deploy-mcp login    Sign in interactively\n  cbgpu-deploy-mcp status   Check authentication\n  cbgpu-deploy-mcp logout   Revoke this login\n  cbgpu-deploy-mcp          Run the stdio MCP server\n\nRequires Node.js 22+. Works with Codex, Claude Code, and other stdio MCP clients.');
 else if(command==='serve')await createServer(api).connect(new StdioServerTransport());
 else throw Error('Unknown command. Run cbgpu-deploy-mcp --help.');
} catch(error){console.error(error.message);process.exitCode=1;}
