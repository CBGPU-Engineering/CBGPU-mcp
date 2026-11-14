import {mkdir, readFile, writeFile, rename, rm, chmod, lstat} from 'node:fs/promises';
import {homedir, platform} from 'node:os';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import lockfile from 'proper-lockfile';
import {PUBLIC_ANON_KEY} from './config.mjs';

export function baseURL(value = 'https://cbgpu.com') {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.pathname !== '/' || url.search || url.hash) throw Error('CBGPU_URL must be an HTTPS origin without credentials or a path.');
  return url.origin;
}
export function sessionPath() {
  const root = platform() === 'win32' ? process.env.LOCALAPPDATA || path.join(homedir(), 'AppData', 'Local') : process.env.XDG_CONFIG_HOME || path.join(homedir(), '.config');
  return path.join(root, 'cbgpu-deploy-mcp', 'session.json');
}
export class ApiError extends Error { constructor(status, message) { super(message); this.status = status; } }
export class DeployAPI {
  constructor({base = process.env.CBGPU_URL || 'https://cbgpu.com', file = sessionPath(), fetcher = fetch, token = process.env.CBGPU_ACCESS_TOKEN, anonKey = process.env.CBGPU_ANON_KEY || PUBLIC_ANON_KEY} = {}) {
    this.base = baseURL(base); this.file = file; this.fetcher = fetcher; this.token = token; this.anonKey = anonKey;
  }
  async json(endpoint, method = 'GET', body, headers = {}) {
    const response = await this.fetcher(this.base + endpoint, {method, headers: {'Content-Type': 'application/json', Origin: this.base, ...headers}, body: body === undefined ? undefined : JSON.stringify(body), redirect: 'error', signal: AbortSignal.timeout(30000)});
    if (response.status === 204 && response.ok) return {};
    const text = await response.text();
    let data; try { data = JSON.parse(text); } catch { throw new ApiError(response.status, 'CBGPU returned a non-JSON response. Check service availability.'); }
    if (!response.ok) throw new ApiError(response.status, response.status === 401 ? 'Login expired or account unavailable. Run cbgpu-deploy-mcp login.' : response.status === 403 ? 'Your account does not have permission for this operation.' : response.status === 429 ? 'CBGPU rate limit reached. Wait before retrying.' : 'CBGPU request failed (' + response.status + '). Check the website project settings.');
    return data;
  }
  async prepare() {
    await mkdir(path.dirname(this.file), {recursive: true, mode: 0o700});
    const dir = await lstat(path.dirname(this.file));
    if (dir.isSymbolicLink() || !dir.isDirectory() || (process.getuid && dir.uid !== process.getuid())) throw Error('Unsafe session directory.');
    if (process.platform !== 'win32') await chmod(path.dirname(this.file), 0o700);
  }
  async read() {
    const stat = await lstat(this.file);
    if (!stat.isFile() || stat.isSymbolicLink() || (process.getuid && stat.uid !== process.getuid()) || (process.platform !== 'win32' && (stat.mode & 0o077))) throw Error('Session file must be private (chmod 600).');
    const session = JSON.parse(await readFile(this.file, 'utf8'));
    if (session.base !== this.base) throw Error('Session belongs to a different CBGPU origin. Log in again.');
    return session;
  }
  async save(session) {
    const temp = this.file + '.' + randomUUID();
    try { await writeFile(temp, JSON.stringify({access_token:session.access_token,refresh_token:session.refresh_token,expires_at:session.expires_at,base:this.base}), {mode: 0o600, flag: 'wx'}); await rename(temp, this.file); }
    finally { await rm(temp, {force: true}); }
  }
  async locked(fn) {
    await this.prepare();
    const release = await lockfile.lock(path.dirname(this.file), {realpath: false, stale: 120000, retries: {retries: 50, minTimeout: 100, maxTimeout: 500}});
    try { return await fn(); } finally { await release(); }
  }
  async login(email, password) {
    return this.locked(async () => {
      const data = await this.json('/backend/auth/v1/token?grant_type=password', 'POST', {email, password}, {apikey: this.anonKey});
      if (!data.access_token || !data.refresh_token) throw Error('No login session returned. Complete account verification on the website.');
      // Verify approval and an active deployment session BEFORE retaining credentials.
      await this.json('/api/deploy/projects', 'GET', undefined, {Authorization: 'Bearer ' + data.access_token});
      await this.save({...data, expires_at: data.expires_at || Math.floor(Date.now()/1000) + data.expires_in});
      return {email: data.user?.email};
    });
  }
  async access(rejectedToken) {
    if (this.token) return this.token;
    return this.locked(async () => {
      let session; try { session = await this.read(); } catch (error) { if (error.code === 'ENOENT') throw Error('Run cbgpu-deploy-mcp login in your terminal first.'); throw error; }
      if (session.expires_at > Date.now()/1000 + 60 && (!rejectedToken || session.access_token !== rejectedToken)) return session.access_token;
      const next = await this.json('/backend/auth/v1/token?grant_type=refresh_token', 'POST', {refresh_token: session.refresh_token}, {apikey: this.anonKey});
      if (!next.access_token || !next.refresh_token) throw Error('Invalid refresh response. Log in again.');
      await this.save({...next, expires_at: next.expires_at || Math.floor(Date.now()/1000) + next.expires_in});
      return next.access_token;
    });
  }
  async request(endpoint, method = 'GET', body) {
    if (!/^\/(projects|servers|compose|teams|domains)(\/[-a-z0-9]+)*$/.test(endpoint)) throw Error('Unsupported deployment endpoint.');
    const token = await this.access();
    try { return await this.json('/api/deploy' + endpoint, method, body, {Authorization: 'Bearer ' + token}); }
    catch (error) {
      if (error.status !== 401 || this.token) throw error;
      const fresh = await this.access(token);
      return this.json('/api/deploy' + endpoint, method, body, {Authorization: 'Bearer ' + fresh});
    }
  }
  async logout() {
    return this.locked(async () => {
      let session; try { session = await this.read(); } catch (error) { if (error.code === 'ENOENT') return; throw error; }
      // Local-only scope: do not sign the user out of the app or other devices.
      const token = session.expires_at > Date.now()/1000 ? session.access_token : await this.json('/backend/auth/v1/token?grant_type=refresh_token', 'POST', {refresh_token: session.refresh_token}, {apikey: this.anonKey}).then(data => data.access_token);
      await this.json('/backend/auth/v1/logout?scope=local', 'POST', {}, {apikey: this.anonKey, Authorization: 'Bearer ' + token});
      await rm(this.file, {force: true});
    });
  }
}

// Never return environment values, auth tokens or embedded repository credentials to an LLM.
export function sanitize(value) {
  if (Array.isArray(value)) return value.map(sanitize);
  if (value && typeof value === 'object') {
    const output = {};
    for (const [key, item] of Object.entries(value)) {
      if (['env_vars','env','secrets'].includes(key)) { output.environment_keys = Object.keys(item || {}); continue; }
      if (/token|password|secret|authorization|apikey/i.test(key) && !['git_connected'].includes(key)) continue;
      output[key] = sanitize(item);
    }
    return output;
  }
  if (typeof value === 'string') return value.replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/g, '$1[REDACTED]@').replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, '[REDACTED JWT]').replace(/\b(?:gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)\b/g, '[REDACTED GitHub token]');
  return value;
}
