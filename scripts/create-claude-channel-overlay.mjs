#!/usr/bin/env node

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

function fail(message) {
  process.stderr.write(`Claude Channel overlay failed: ${message}\n`)
  process.exit(2)
}

function validateServerDefinition(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    fail('server definition must be a JSON object')
  }
  if (typeof value.command !== 'string' || !value.command.trim()) {
    fail('server definition requires a command')
  }
  if (value.args != null && (!Array.isArray(value.args) || value.args.some(item => typeof item !== 'string'))) {
    fail('server definition args must be an array of strings')
  }
  if (value.env != null && (
    typeof value.env !== 'object' ||
    Array.isArray(value.env) ||
    Object.values(value.env).some(item => typeof item !== 'string')
  )) {
    fail('server definition env must map names to strings')
  }
  const allowedEnvironment = new Set([
    'OPS_BRAIN_LIVE_URL',
    'OPS_BRAIN_LIVE_LABEL',
    'OPS_BRAIN_EXPECTED_AGENT',
    'OPS_BRAIN_AGENT_TOKEN_FILE',
    'OPS_BRAIN_AGENT_TOKEN_HELPER_JSON',
  ])
  const unsupported = Object.keys(value.env ?? {}).find(name => !allowedEnvironment.has(name))
  if (unsupported) {
    fail(`server definition environment key is not allowed: ${unsupported}`)
  }
  return value
}

function assertEmptyPrivateDirectory(directory) {
  const resolved = path.resolve(directory)
  let stat
  try {
    stat = fs.lstatSync(resolved)
  } catch {
    fail(`overlay directory does not exist: ${resolved}`)
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail(`overlay path must be a real directory: ${resolved}`)
  }
  if (process.platform !== 'win32' && (stat.mode & 0o077) !== 0) {
    fail(`overlay directory must not be accessible to group or other users: ${resolved}`)
  }
  if (fs.readdirSync(resolved).length !== 0) {
    fail(`overlay directory must be empty: ${resolved}`)
  }
  return resolved
}

function mirrorConfigDirectory(baseDirectory, overlayDirectory) {
  if (!fs.existsSync(baseDirectory)) return
  const stat = fs.lstatSync(baseDirectory)
  if (!stat.isDirectory()) fail(`Claude config path is not a directory: ${baseDirectory}`)

  for (const entry of fs.readdirSync(baseDirectory, { withFileTypes: true })) {
    if (entry.name === '.claude.json') continue
    const source = path.join(baseDirectory, entry.name)
    const target = path.join(overlayDirectory, entry.name)
    if (entry.isDirectory()) {
      fs.symlinkSync(source, target, process.platform === 'win32' ? 'junction' : 'dir')
    } else if (entry.isFile()) {
      mirrorConfigFile(source, target)
    } else if (entry.isSymbolicLink()) {
      let resolved
      let resolvedStat
      try {
        resolved = fs.realpathSync(source)
        resolvedStat = fs.statSync(resolved)
      } catch {
        continue
      }
      if (resolvedStat.isDirectory()) {
        fs.symlinkSync(resolved, target, process.platform === 'win32' ? 'junction' : 'dir')
      } else {
        mirrorConfigFile(resolved, target)
      }
    }
  }
}

function mirrorConfigFile(source, target) {
  if (process.platform !== 'win32') {
    fs.symlinkSync(source, target, 'file')
    return
  }
  try {
    fs.linkSync(source, target)
  } catch (error) {
    fail(`cannot mirror Claude state file without copying it: ${source} (${error.code ?? 'unknown error'})`)
  }
}

const [overlayArg, serverName, serverJson] = process.argv.slice(2)
if (!overlayArg || !serverName || !serverJson) {
  fail('usage: create-claude-channel-overlay.mjs OVERLAY_DIR SERVER_NAME SERVER_JSON')
}
if (!/^[A-Za-z0-9._-]{1,80}$/.test(serverName)) fail('server name is invalid')

let serverDefinition
try {
  serverDefinition = validateServerDefinition(JSON.parse(serverJson))
} catch (error) {
  if (error instanceof SyntaxError) fail('server definition is not valid JSON')
  throw error
}

const overlayDirectory = assertEmptyPrivateDirectory(overlayArg)
const explicitBase = process.env.CLAUDE_CONFIG_DIR?.trim()
const baseDirectory = path.resolve(explicitBase || path.join(os.homedir(), '.claude'))
mirrorConfigDirectory(baseDirectory, overlayDirectory)
const config = { mcpServers: { [serverName]: serverDefinition } }

const overlayConfig = path.join(overlayDirectory, '.claude.json')
fs.writeFileSync(overlayConfig, `${JSON.stringify(config, null, 2)}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
