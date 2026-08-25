import { closeSync, constants, fstatSync, openSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'

const LABEL_RE = /^[A-Za-z0-9._-]+$/
const AGENT_RE = /^[A-Za-z0-9._-]+$/

export function loadConfig(env = process.env) {
  const token = loadToken(env)
  if (/\r|\n/.test(token)) {
    throw new Error('OPS_BRAIN_AGENT_TOKEN must be a single line')
  }

  const url = parseLiveUrl(required(env.OPS_BRAIN_LIVE_URL, 'OPS_BRAIN_LIVE_URL'))
  const expectedAgent = required(env.OPS_BRAIN_EXPECTED_AGENT, 'OPS_BRAIN_EXPECTED_AGENT')
  if (Buffer.byteLength(expectedAgent) > 80 || !AGENT_RE.test(expectedAgent)) {
    throw new Error(
      "OPS_BRAIN_EXPECTED_AGENT must be 1-80 bytes using only letters, digits, '.', '_' or '-'",
    )
  }
  const label = (env.OPS_BRAIN_LIVE_LABEL ?? 'claude-code').trim()
  if (!label || Buffer.byteLength(label) > 80 || !LABEL_RE.test(label)) {
    throw new Error(
      "OPS_BRAIN_LIVE_LABEL must be 1-80 bytes using only letters, digits, '.', '_' or '-'",
    )
  }

  return Object.freeze({ url: url.toString(), token, label, expectedAgent })
}

function loadToken(env) {
  const inline = env.OPS_BRAIN_AGENT_TOKEN?.trim()
  const file = env.OPS_BRAIN_AGENT_TOKEN_FILE?.trim()
  const helper = env.OPS_BRAIN_AGENT_TOKEN_HELPER_JSON?.trim()
  if ([inline, file, helper].filter(Boolean).length > 1) {
    throw new Error('set only one agent token source: inline, file, or helper')
  }
  if (helper) return loadTokenFromHelper(helper)
  if (!file) return required(inline, 'OPS_BRAIN_AGENT_TOKEN')

  let descriptor
  try {
    const flags = constants.O_RDONLY | (process.platform === 'win32' ? 0 : constants.O_NOFOLLOW)
    descriptor = openSync(file, flags)
    const stats = fstatSync(descriptor)
    if (!stats.isFile() || (process.platform !== 'win32' && (stats.mode & 0o077) !== 0)) {
      throw new Error('unsafe token file')
    }
    return required(readFileSync(descriptor, 'utf8').trim(), 'OPS_BRAIN_AGENT_TOKEN_FILE')
  } catch {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_FILE must be a regular file inaccessible to group/other')
  } finally {
    if (descriptor !== undefined) closeSync(descriptor)
  }
}

function loadTokenFromHelper(raw) {
  let command
  try { command = JSON.parse(raw) } catch { throw new Error('OPS_BRAIN_AGENT_TOKEN_HELPER_JSON must be JSON') }
  if (
    !Array.isArray(command) || command.length < 1 || command.length > 32 ||
    command.some(value => typeof value !== 'string' || !value || /[\r\n\0]/.test(value))
  ) {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_HELPER_JSON must be an array of safe command strings')
  }
  try {
    const value = execFileSync(command[0], command.slice(1), {
      encoding: 'utf8', input: '', timeout: 15_000, maxBuffer: 16_384,
      windowsHide: true,
    }).trim()
    return required(value, 'agent token helper output')
  } catch {
    throw new Error('agent token helper failed')
  }
}

function required(value, name) {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new Error(`${name} is required`)
  }
  return value.trim()
}

function parseLiveUrl(value) {
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error('OPS_BRAIN_LIVE_URL must be a valid WebSocket URL')
  }
  if (url.username || url.password || url.search || url.hash) {
    throw new Error('OPS_BRAIN_LIVE_URL must not contain credentials, query parameters, or a fragment')
  }
  if (url.pathname !== '/live') {
    throw new Error('OPS_BRAIN_LIVE_URL path must be exactly /live')
  }
  if (url.protocol === 'wss:') return url
  if (url.protocol !== 'ws:' || !isLoopback(url.hostname)) {
    throw new Error('OPS_BRAIN_LIVE_URL must use wss:// (ws:// is allowed only for loopback testing)')
  }
  return url
}

function isLoopback(hostname) {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]'
}
