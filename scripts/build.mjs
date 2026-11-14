import {build} from 'esbuild';
import {builtinModules} from 'node:module';
import {readFile,writeFile,mkdir,chmod,readdir} from 'node:fs/promises';
import path from 'node:path';
const result=await build({entryPoints:['src/cli.mjs'],bundle:true,platform:'node',target:'node22',format:'esm',outfile:'dist/cli.mjs',metafile:true,legalComments:'linked',banner:{js:'import { createRequire as __cbgpuCreateRequire } from "node:module"; const require = __cbgpuCreateRequire(import.meta.url);'}});
const packages=new Map();
for(const name of Object.keys(result.metafile.inputs)){
 if(!name.includes('node_modules/'))continue;
 let dir=path.dirname(path.resolve(name));
 while(dir!==path.dirname(dir)){
  try{const pkg=JSON.parse(await readFile(path.join(dir,'package.json'),'utf8'));if(pkg.name&&pkg.version){packages.set(dir,pkg);break;}}catch{}
  dir=path.dirname(dir);
 }
}
let notices='Bundled third-party software notices\n====================================\n';
for(const [dir,pkg]of [...packages].sort((a,b)=>a[1].name.localeCompare(b[1].name))){
 notices+='\n'+pkg.name+' '+pkg.version+'\nLicense: '+JSON.stringify(pkg.license||'See license text')+'\n';
 const files=(await readdir(dir)).filter(n=>/^(licen[sc]e|copying|notice)(\.|$)/i.test(n));
 if(!files.length)throw Error('Missing license text for bundled dependency '+pkg.name);
 for(const name of files)notices+='\n'+name+'\n'+await readFile(path.join(dir,name),'utf8')+'\n';
}
await mkdir('dist',{recursive:true});await writeFile('dist/THIRD_PARTY_NOTICES.txt',notices);await chmod('dist/cli.mjs',0o755);
// Packaged CLI must not depend on external npm subpath exports at runtime.
for(const output of Object.values(result.metafile.outputs))for(const entry of output.imports)if(entry.external&&!entry.path.startsWith('node:')&&!builtinModules.includes(entry.path))throw Error('Unexpected external runtime import: '+entry.path);
console.error('Built standalone deployment MCP with notices for '+packages.size+' bundled dependencies.');
