#!/usr/bin/env node

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

function fail(message) {
  process.stderr.write(`Claude Channel overlay failed: ${message}\n`)
  process.exit(2)
}

function readJsonObject(file, description) {
  if (!fs.existsSync(file)) return {}
  let parsed
  try {
    parsed = JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch {
    fail(`${description} is not valid JSON: ${file}`)
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    fail(`${description} must contain a JSON object: ${file}`)
  }
  return parsed
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
  if (Object.hasOwn(value.env ?? {}, 'OPS_BRAIN_AGENT_TOKEN')) {
    fail('server definition must pass a token file or credential helper, never a bearer')
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
    if (!['EXDEV', 'EPERM', 'EACCES'].includes(error.code)) throw error
    fs.copyFileSync(source, target, fs.constants.COPYFILE_EXCL)
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
const userConfig = path.resolve(
  explicitBase ? path.join(baseDirectory, '.claude.json') : path.join(os.homedir(), '.claude.json'),
)

mirrorConfigDirectory(baseDirectory, overlayDirectory)
const config = readJsonObject(userConfig, 'Claude user config')
const existingServers = config.mcpServers
if (existingServers != null && (typeof existingServers !== 'object' || Array.isArray(existingServers))) {
  fail(`Claude user config mcpServers must be an object: ${userConfig}`)
}
config.mcpServers = { ...(existingServers ?? {}), [serverName]: serverDefinition }

const overlayConfig = path.join(overlayDirectory, '.claude.json')
fs.writeFileSync(overlayConfig, `${JSON.stringify(config, null, 2)}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
