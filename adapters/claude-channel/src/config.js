const LABEL_RE = /^[A-Za-z0-9._-]+$/

export function loadConfig(env = process.env) {
  const token = required(env.OPS_BRAIN_AGENT_TOKEN, 'OPS_BRAIN_AGENT_TOKEN')
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
