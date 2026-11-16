# CBGPU Deploy MCP

[![CI](https://github.com/CBGPU-Engineering/CBGPU-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/CBGPU-Engineering/CBGPU-mcp/actions/workflows/ci.yml)
[![Node.js](https://img.shields.io/badge/Node.js-22%2B-339933?logo=nodedotjs&logoColor=white)](https://nodejs.org/)
[![MCP](https://img.shields.io/badge/MCP-stdio-111111)](https://modelcontextprotocol.io/)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-D22128)](./LICENSE)

## Native Rust candidate

A native Rust implementation now provides the same 17 deployment/environment
tools and stdio protocol. It does not invoke the TypeScript server, Node.js or a
Python helper. The currently published **v1.1.0 npm entry remains unchanged**
until native release artifacts pass the Linux, macOS and Windows CI checks.
There is no automatic download or npm publication in the candidate build.

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
target/release/cbgpu-deploy-mcp --help
target/release/cbgpu-deploy-mcp login
```

On Windows the executable ends in `.exe`. The native binary needs no Node.js;
Node.js is used only by the optional compatibility launcher and SDK tests.

The native client retains the existing origin-bound `session.json` format,
configuration variables and lock-directory protocol, so a legacy client and a
native client serialize refreshes instead of racing. Unix session files are
owner-only; the Windows candidate protects the session directory/file ACL and
uses an atomic Windows replacement for credential updates. Passwords remain
interactive, hidden and unsaved. Like the current release, the native candidate
does not invent a browser/device OAuth endpoint that the deployment API does
not currently provide.

For a local thin-launcher candidate and actual SDK interoperability tests:

```bash
npm ci --ignore-scripts
npm run build:native
node dist/native-cli.mjs --help
node test/native-stdio.ts target/release/cbgpu-deploy-mcp
```

`build:native` compiles only this computer's executable and records its SHA-256
in `dist/native/manifest.json`. This is not a cross-platform release package;
verified release artifacts must be assembled before changing the published npm
entry. An explicit absolute `CBGPU_MCP_NATIVE_BINARY` may select a locally built
binary for the candidate launcher. Build outputs and credentials stay outside Git.

## Ask your coding agent to set it up

Copy this into Codex or Claude Code **on the computer where you use that agent**:

```text
Install and configure the official CBGPU deployment MCP for this coding agent.
Read https://github.com/CBGPU-Engineering/CBGPU-mcp/blob/main/README.md first.
Check this computer's OS, Node.js (22+) and available coding-agent CLI.
Use the pinned v1.1.0 GitHub release package from that README.
Register a USER-SCOPED stdio MCP named cbgpu-deploy for the coding agent I am using;
keep existing MCP settings. Use an absolute executable path if necessary.
Run the separate login command in an interactive terminal so I can enter my own
CBGPU email and password privately. Never ask for a password in chat, put one in
arguments/config, read an existing browser session, or print saved credentials.
If you cannot offer an interactive terminal, show me the login command to run.
Use the status command and MCP list_projects for frontend and server to verify.
Do not create, change, redeploy or delete projects during installation.
After setup, report whether login succeeded and whether the agent needs restarting.
When I later ask to deploy, inspect my repository and the existing CBGPU projects,
select the correct branch and root directory, then create or update that project.
Poll status and logs until it is ready or has failed; accepted/queued is not success.
When I authorize an env file for deployment, use set_environment_from_file with its absolute path. Never print its contents or put login credentials in tool arguments.
```

CBGPU Deploy MCP lets Codex, Claude Code and other MCP clients deploy GitHub
frontends, backend servers, Docker images and Docker Compose stacks through the
same authenticated APIs as the [CBGPU deployment console](https://researcher.cbgpu.com/deploy).
The MCP runs locally over stdio; deployments run on CBGPU.
The release bundles its runtime dependencies, so users do not need a separate
MCP SDK installation. Bundled licenses are included in `dist/THIRD_PARTY_NOTICES.txt`.

## Connect manually

Requires Node.js 22+ and an approved CBGPU account with an email/password login.
For a Google-only account, set an account password using CBGPU's password recovery
flow first. This release does not implement browser OAuth login.

### 1. Sign in once

Run this in your own interactive terminal. The password is hidden and is never saved:

```bash
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp login
```

### 2. Register with your coding agent

**Codex:**

```bash
codex mcp add cbgpu-deploy -- npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp
```

**Claude Code:**

```bash
claude mcp add --scope user cbgpu-deploy -- npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp
```

Restart the agent or reconnect its MCP servers after registering. On Windows,
if the client cannot launch `npx`, use `cmd /c npx` as its command, or use the
absolute path to `npx.cmd`. Windows users should restrict the session directory
to their own account using the Windows file permissions dialog; Unix modes do
not provide Windows ACL enforcement.

**Other stdio MCP clients:**

```json
{
  "mcpServers": {
    "cbgpu-deploy": {
      "command": "npx",
      "args": [
        "--yes",
        "--package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz",
        "cbgpu-deploy-mcp"
      ]
    }
  }
}
```

### 3. Ask for a deployment

```text
Deploy this repository's frontend folder to CBGPU. Check existing projects first,
use the main branch and the frontend folder as root, and show me the production
URL after the deployment is ready. Do not expose environment secrets.
```

```text
Deploy the server/ Docker Compose stack to CBGPU with 3 CPU cores, 4 GB RAM and
40 GB persistent storage. Use api-gw as the gateway service and container port
8000. Make it public at my-app.server.cbgpu.com. Keep volumes on redeploy and
inspect health diagnostics if a service fails.
```

Push local changes to GitHub before asking to redeploy: builds fetch the remote
branch, not uncommitted files on your computer. Frontend production URLs use
`<slug>.front.cbgpu.com`; server URLs use `<slug>.server.cbgpu.com`.
Private GitHub frontends can use a repository token configured in the website.
Environment configuration is available through set_environment and set_environment_from_file. Prefer the file tool so secret values stay out of chat and tool arguments; results return variable names only.

## Tools

| Tool | Purpose |
| --- | --- |
| `list_projects` | Owned and shared frontend/server projects |
| `get_project` | Settings, state and frontend deployment history |
| `create_frontend` | GitHub repository, branch, root folder and build settings |
| `update_frontend` | Change settings without starting a build |
| `create_server` | Git, Docker image or Docker Compose; up to 4 cores and 16 GB RAM |
| `update_server` | Change resources/settings while preserving existing environment values |
| `redeploy` | Fetch the latest configured Git branch and request a new deployment |
| `deployment_logs` | Frontend logs, Compose health/failure diagnostics, or available server log fields |
| `rollback_frontend` | Promote a retained successful frontend deployment |
| `set_server_domain` | Choose a server subdomain; public scope must be set separately |
| `list_domains` | Custom domain state and DNS instructions |
| `add_domain` | Attach a custom domain; does not change DNS at your registrar |
| `verify_domain` | Check the domain ID returned by `add_domain`/`list_domains` |
| `list_collaborators` | Members and current account role |
| `list_environment` | Variable names only |
| `set_environment` | Add/update selected variables; null removes a variable |
| `set_environment_from_file` | Import the specifically authorized local dotenv file without returning values |

Server `settings.runtime` is the legacy compatibility field. For newer runtimes,
use `settings.runtime_preset`: `node22`, `node20`, `node18`, `python312`,
`python311`, `python39`, `java21`, `java17`, `go1`, or `dotnet10`.
`settings.root_directory` selects a subfolder for Git and Compose projects.
Compose additionally accepts `compose_file`, `gateway_service` and `storage_gb`
(40–200 GB). Existing website validation and capacity limits still apply.
Some non-Compose projects do not expose runtime logs through the public API;
this MCP reports that explicitly rather than fabricating logs or readiness.

## Environment variables from an agent or CLI

Ask your agent: “Use this project's `.env.production` to configure my CBGPU server.
Keep other variables, then redeploy and check its logs.” The agent can call
`set_environment_from_file` with `kind`, `project_id` and the absolute local
`env_file` path. The file is read on the **computer running the MCP**, parsed
locally and sent over HTTPS to the existing authenticated deployment API. The
tool returns updated/removed variable names, never their values.

For a few non-secret values, `set_environment` accepts `variables` as an object;
set a key to `null` to remove that variable. Project owners and editors can change
environment variables; viewers cannot. Existing keys are preserved. CBGPU's
internal `CBGPU_` configuration remains managed through project settings tools.
Both `create_frontend` and `create_server` also accept an optional `env_file`
absolute path, so required variables are present on the **first** deployment.
File imports do not delete keys absent from that file. Dotenv quotes, comments,
empty strings and multiline values are supported; `${VAR}` and `$(command)` are
literal strings, with **no shell execution or environment expansion**.

Direct CLI, after login:

```bash
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp env list server PROJECT_ID
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp env set server PROJECT_ID --file .env.production
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp env unset server PROJECT_ID --keys OLD_KEY,UNUSED_KEY
```

Use `frontend` instead of `server` for frontend projects. Saved environment changes
apply on the **next deployment**; call `redeploy` when ready. The server API uses
its existing read/merge/write behavior, so avoid simultaneous environment edits
from multiple clients. Upload only the file and keys intended for this project.

## Sessions and permissions

The website checks account approval, active login sessions and existing project
owner/editor/viewer permissions for every request. No internal deployment key,
service-role key or arbitrary shell access is given to the agent.
A login is a normal CBGPU account session, **not a separately scoped API token**.
It is stored locally outside repositories and automatically refreshed, with a
cross-process lock so Codex and Claude Code can run together. The MCP only exposes
the deployment tools listed above. Treat the session file as sensitive.

- Linux/macOS: `$XDG_CONFIG_HOME/cbgpu-deploy-mcp/session.json`, default `~/.config/cbgpu-deploy-mcp/session.json`.
- Windows: `%LOCALAPPDATA%\cbgpu-deploy-mcp\session.json`.
- Unix directory/file permissions: `700` / `600`; writes are atomic.
- Environment values and credentials are omitted from tool results. Logs remain
  untrusted user build output; do not print secrets in your application builds.
- Network timeouts do not automatically retry deployment mutations, avoiding
  accidental duplicate builds. Only a definitive authentication rejection is retried after refresh.

Check authentication or revoke **only this MCP login**, leaving browser/app sessions alone:

```bash
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp status
npx --yes --package=https://github.com/CBGPU-Engineering/CBGPU-mcp/releases/download/v1.1.0/cbgpu-deploy-mcp-1.1.0.tgz cbgpu-deploy-mcp logout
```

`CBGPU_ACCESS_TOKEN` optionally supplies an existing short-lived user JWT instead
of using saved login. It cannot refresh itself. Never put it in a committed MCP
configuration. `CBGPU_URL` defaults to `https://cbgpu.com` and must be an HTTPS
origin; changing it requires a separate login. `CBGPU_ANON_KEY` can override the
public browser key for a separately operated CBGPU instance. This package contains
only CBGPU's public publishable browser key, not a privileged server key.

## Development

```bash
npm ci
npm run check
npm test
npm run build
npm run test:package
npm audit
npm pack --dry-run
```

Tests use the actual MCP SDK transport and cover tool routing, validation,
Compose diagnostics, environment preservation, permissions, refresh races,
private session storage, redirect refusal and logout. CI runs on Linux, macOS
and Windows with Node.js 22 and 24. The packed release is installed into a fresh
temporary directory and tested without an external SDK installation. No real user deployments are created by these tests. Environment tests also verify dotenv parsing, owner/editor checks, viewer rejection, preservation of system settings and secret-free results.

Sources: [MCP SDK](https://github.com/modelcontextprotocol/typescript-sdk),
[Codex MCP](https://developers.openai.com/codex/mcp/),
[Claude Code MCP](https://code.claude.com/docs/en/mcp).

## License

Licensed under the [Apache License 2.0](./LICENSE).


## Development

The CLI, MCP server, tests and packaging tools are written in TypeScript.
Use Node.js 22.18 or newer. Run `npm ci`, `npm run check`, `npm test`,
`npm run build` and `npm run test:package`. Strict type checking covers the
source, tests and build scripts. CI checks Linux, macOS and Windows on Node.js
22 and 24 and rejects authored JavaScript sources.

The published command is generated by esbuild as a standalone Node.js bundle;
its JavaScript is a build artifact, not maintained application source. The
packaged command does not depend on TypeScript stripping in `node_modules`.
