import { readFileSync, statSync } from 'node:fs'

const LABEL_RE = /^[A-Za-z0-9._-]+$/

export function loadConfig(env = process.env) {
  const token = loadToken(env)
  if (/\r|\n/.test(token)) {
    throw new Error('OPS_BRAIN_AGENT_TOKEN must be a single line')
  }

  const url = parseLiveUrl(required(env.OPS_BRAIN_LIVE_URL, 'OPS_BRAIN_LIVE_URL'))
  const label = (env.OPS_BRAIN_LIVE_LABEL ?? 'claude-code').trim()
  if (!label || Buffer.byteLength(label) > 80 || !LABEL_RE.test(label)) {
    throw new Error(
      "OPS_BRAIN_LIVE_LABEL must be 1-80 bytes using only letters, digits, '.', '_' or '-'",
    )
  }

  return Object.freeze({ url: url.toString(), token, label })
}

function loadToken(env) {
  const inline = env.OPS_BRAIN_AGENT_TOKEN?.trim()
  const file = env.OPS_BRAIN_AGENT_TOKEN_FILE?.trim()
  if (inline && file) {
    throw new Error('set only one of OPS_BRAIN_AGENT_TOKEN or OPS_BRAIN_AGENT_TOKEN_FILE')
  }
  if (!file) return required(inline, 'OPS_BRAIN_AGENT_TOKEN')

  let stats
  let value
  try {
    stats = statSync(file)
    value = readFileSync(file, 'utf8').trim()
  } catch {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_FILE must be a readable regular file')
  }
  if (!stats.isFile() || (stats.mode & 0o077) !== 0) {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_FILE must be a regular file inaccessible to group/other')
  }
  return required(value, 'OPS_BRAIN_AGENT_TOKEN_FILE')
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
