import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { WebSocketServer } from 'ws'

const MAIN = join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'main.js')
const BOUND_PEER = '019cd123-1234-7123-8123-1234567890ff'

// Serves one /live connection that registers the peer under `agentName`,
// letting the test drive the adapter's real registration path end to end.
async function liveServerBoundTo(agentName) {
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' })
  await once(server, 'listening')
  server.on('connection', socket => {
    socket.on('message', raw => {
      const frame = JSON.parse(raw.toString())
      if (frame.type !== 'register') return
      socket.send(JSON.stringify({
        type: 'registered',
        protocol_version: 1,
        peer: {
          peer_id: BOUND_PEER,
          agent_name: agentName,
          adapter: 'claude_code',
          label: 'claude-test',
          metadata_trust: 'self_reported',
        },
      }))
    })
  })
  return server
}

function startAdapter({ port, expectedAgent, stateDir }) {
  const child = spawn(process.execPath, [MAIN], {
    env: {
      ...process.env,
      OPS_BRAIN_LIVE_URL: `ws://127.0.0.1:${port}/live`,
      OPS_BRAIN_EXPECTED_AGENT: expectedAgent,
      OPS_BRAIN_LIVE_LABEL: 'claude-test',
      OPS_BRAIN_AGENT_TOKEN: 'fixture-token-not-a-secret',
      OPS_BRAIN_LIVE_STATE_DIR: stateDir,
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  // The adapter registers with /live only after the MCP client initializes, so
  // a bare spawn would never reach the code under test.
  const send = value => child.stdin.write(`${JSON.stringify(value)}\n`)
  send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'adapter-test', version: '1.0.0' },
    },
  })
  send({ jsonrpc: '2.0', method: 'notifications/initialized' })
  return child
}

async function waitForLog(stateDir, pattern, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    let body = ''
    try {
      for (const name of readdirSync(stateDir)) {
        if (name.startsWith('claude-adapter.')) body += readFileSync(join(stateDir, name), 'utf8')
      }
    } catch {
      // The directory may not exist yet on the first poll.
    }
    if (pattern.test(body)) return body
    await new Promise(resolve => setTimeout(resolve, 50))
  }
  throw new Error(`log never matched ${pattern} within ${timeoutMs}ms`)
}

test('records a terminal identity mismatch in the adapter log', async t => {
  const stateDir = mkdtempSync(join(tmpdir(), 'ops-brain-main-'))
  const server = await liveServerBoundTo('CC-Somebody-Else')
  const child = startAdapter({ port: server.address().port, expectedAgent: 'CC-Stealth', stateDir })
  t.after(async () => {
    child.kill('SIGKILL')
    await new Promise(resolve => server.close(resolve))
    rmSync(stateDir, { recursive: true, force: true })
  })

  // Claude Code discards this process's stderr, so this file is the only place
  // an operator can learn the channel never bound.
  const body = await waitForLog(stateDir, /"retryable":false/)
  assert.match(body, /bound identity does not match/)
  assert.match(body, /CC-Stealth/)
  const fatal = body.trim().split('\n').map(line => JSON.parse(line)).find(r => r.retryable === false)
  assert.equal(fatal.level, 'error')
  assert.equal(typeof fatal.ts, 'string')
  // A retryable-looking disconnect after a terminal error would contradict it.
  const disconnect = body.trim().split('\n').map(line => JSON.parse(line))
    .find(r => r.message === 'live adapter disconnected')
  if (disconnect) assert.equal(disconnect.will_reconnect, false)
  // The bearer must never reach the log.
  assert.doesNotMatch(body, /fixture-token-not-a-secret/)
})

test('records a successful bind, and never the bearer', async t => {
  const stateDir = mkdtempSync(join(tmpdir(), 'ops-brain-main-'))
  const server = await liveServerBoundTo('CC-Stealth')
  const child = startAdapter({ port: server.address().port, expectedAgent: 'CC-Stealth', stateDir })
  t.after(async () => {
    child.kill('SIGKILL')
    await new Promise(resolve => server.close(resolve))
    rmSync(stateDir, { recursive: true, force: true })
  })

  const body = await waitForLog(stateDir, /live adapter connected/)
  assert.match(body, /claude channel adapter started/)
  const connected = body.trim().split('\n').map(line => JSON.parse(line))
    .find(r => r.message === 'live adapter connected')
  assert.equal(connected.agent_name, 'CC-Stealth')
  assert.equal(connected.peer_id, BOUND_PEER)
  assert.doesNotMatch(body, /fixture-token-not-a-secret/)
})

test('records an owned disconnect before a graceful shutdown closes the log', async t => {
  const stateDir = mkdtempSync(join(tmpdir(), 'ops-brain-main-'))
  const server = await liveServerBoundTo('CC-Stealth')
  const child = startAdapter({ port: server.address().port, expectedAgent: 'CC-Stealth', stateDir })
  t.after(async () => {
    if (child.exitCode === null) child.kill('SIGKILL')
    await new Promise(resolve => server.close(resolve))
    rmSync(stateDir, { recursive: true, force: true })
  })

  await waitForLog(stateDir, /live adapter connected/)
  child.kill('SIGTERM')
  await once(child, 'exit')
  const body = await waitForLog(stateDir, /live adapter disconnected/)
  const disconnected = body.trim().split('\n').map(line => JSON.parse(line))
    .find(record => record.message === 'live adapter disconnected')
  assert.equal(disconnected.close_code, 1000)
  assert.equal(disconnected.reason, 'adapter stopping')
  assert.equal(disconnected.will_reconnect, false)
})

test('records a startup failure that aborts before the first connection', async t => {
  const stateDir = mkdtempSync(join(tmpdir(), 'ops-brain-main-'))
  t.after(() => rmSync(stateDir, { recursive: true, force: true }))
  const child = spawn(process.execPath, [MAIN], {
    env: {
      ...process.env,
      OPS_BRAIN_LIVE_URL: 'wss://ops-brain.example/live?leak=1',
      OPS_BRAIN_EXPECTED_AGENT: 'CC-Stealth',
      OPS_BRAIN_AGENT_TOKEN: 'fixture-token-not-a-secret',
      OPS_BRAIN_LIVE_STATE_DIR: stateDir,
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  const [code] = await once(child, 'exit')

  // This is the class of failure that previously vanished completely: the
  // logger is built before loadConfig precisely so it is recorded.
  assert.equal(code, 1)
  const body = await waitForLog(stateDir, /Claude Channel failed/, 1_000)
  assert.match(body, /query parameters/)
  assert.doesNotMatch(body, /fixture-token-not-a-secret/)
})
