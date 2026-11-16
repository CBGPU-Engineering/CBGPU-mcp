import type {DeploymentClient} from './api.ts';
import {open,lstat} from 'node:fs/promises';
import {constants} from 'node:fs';
import path from 'node:path';
import {parse} from 'dotenv';
const projectID=/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i;
export function environmentChanges(kind: string,changes: unknown): Record<string,string|null> {
 if(!['frontend','server'].includes(kind)||!changes||typeof changes!=='object'||Array.isArray(changes))throw Error('Invalid environment changes.');
 const entries=Object.entries(changes);
 if(!entries.length||entries.length>(kind==='frontend'?40:160))throw Error('Environment changes exceed the project variable limit or are empty.');
 for(const [key,value]of entries){
  if(!/^[A-Za-z_][A-Za-z0-9_]{0,99}$/.test(key)||key.startsWith('CBGPU_')||['__proto__','prototype','constructor'].includes(key))throw Error('Invalid or reserved environment variable name. CBGPU_ settings use the project settings tools.');
  if(value!==null&&(typeof value!=='string'||value.length>8192||value.includes('\0')))throw Error('Environment values must be strings up to 8192 characters, or null to remove a variable.');
 }
 if(Buffer.byteLength(JSON.stringify(changes))>60000)throw Error('Environment payload exceeds the API request limit.');
 return Object.fromEntries(entries) as Record<string,string|null>;
}
export async function environmentFile(filename: string) {
 if(typeof filename!=='string'||!path.isAbsolute(filename)||!/(?:^\.env(?:\..*)?$|\.env$)/.test(path.basename(filename)))throw Error('Provide an absolute path to an explicitly chosen .env file.');
 let file;
 try{
  if((await lstat(filename)).isSymbolicLink())throw Error();
  file=await open(filename,constants.O_RDONLY|(constants.O_NOFOLLOW||0));const stat=await file.stat();
  if(!stat.isFile()||stat.size>131072)throw Error();
  const buffer=Buffer.alloc(131073);const {bytesRead}=await file.read(buffer,0,buffer.length,0);if(bytesRead>131072)throw Error();
  const values=parse(buffer.subarray(0,bytesRead));if(!Object.keys(values).length)throw Error();return values;
 }catch{throw Error('Cannot read the selected .env file. It must be a regular non-symlink file, at most 128 KiB, containing environment assignments.');}
 finally{await file?.close();}
}
function endpoint(kind: string,id: string){if(!['frontend','server'].includes(kind)||!projectID.test(id))throw Error('Invalid deployment kind or project ID.');return kind==='frontend'?'/projects/'+id:'/servers/projects/'+id;}
export async function listEnvironment(api: DeploymentClient,kind: string,id: string){
 const {project}=await api.request(endpoint(kind,id));
 return {project_id:id,kind,environment_keys:[...new Set<string>([...(project.environment_keys||[]),...Object.keys(project.env_vars||{})])].filter(key=>!key.startsWith('CBGPU_')).sort()};
}
export async function setEnvironment(api: DeploymentClient,kind: string,id: string,changes: unknown){
 const target=endpoint(kind,id),values=environmentChanges(kind,changes);
 const {project}=await api.request(target);
 if(!['owner','editor'].includes(project.collaboration_role))throw Error('Editing environment variables requires project owner or editor access.');
 let payload;
 if(kind==='frontend'){const keys=new Set(project.environment_keys||[]);for(const [key,value]of Object.entries(values)){if(value===null)keys.delete(key);else keys.add(key);}if(keys.size>40)throw Error('Frontend environment variable limit exceeded.');payload={env:values};}
 else{
  const env={...project.env_vars};for(const [key,value]of Object.entries(values)){if(value===null)delete env[key];else env[key]=value;}
  if(Object.keys(env).length>160)throw Error('Server environment variable limit exceeded.');payload={env_vars:env};
 }
 if(Buffer.byteLength(JSON.stringify(payload))>64000)throw Error('Merged environment exceeds the API request limit.');
 // Deliberately never return the API response: server records may contain env_vars.
 await api.request(target,'PATCH',payload);
 return {ok:true,project_id:id,kind,updated_keys:Object.entries(values).filter(([,v])=>v!==null).map(([key])=>key).sort(),removed_keys:Object.entries(values).filter(([,v])=>v===null).map(([key])=>key).sort(),redeploy_required:true};
}
export async function environmentCommand(api: DeploymentClient,args: string[]){
 const [action,kind,id,...options]=args;
 if(action==='list'&&options.length===0)return listEnvironment(api,kind,id);
 if(action==='set'&&options.length===2&&options[0]==='--file')return setEnvironment(api,kind,id,await environmentFile(path.resolve(options[1])));
 if(action==='unset'&&options.length===2&&options[0]==='--keys')return setEnvironment(api,kind,id,Object.fromEntries(options[1].split(',').filter(Boolean).map(key=>[key,null])));
 throw Error('Usage: cbgpu-deploy-mcp env list frontend|server PROJECT_ID; env set frontend|server PROJECT_ID --file .env.production; env unset frontend|server PROJECT_ID --keys KEY1,KEY2');
}
