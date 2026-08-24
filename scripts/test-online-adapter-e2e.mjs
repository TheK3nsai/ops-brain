#!/usr/bin/env node

import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'
import { spawn } from 'node:child_process'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(scriptDirectory, '..')
const requireFromAdapter = createRequire(path.join(root, 'adapters', 'claude-channel', 'package.json'))
const { WebSocketServer } = requireFromAdapter('ws')

function withTimeout(promise, description, timeoutMs = 5_000) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`timed out waiting for ${description}`)), timeoutMs)
    }),
  ]).finally(() => clearTimeout(timer))
}

function jsonLineReader(stream) {
  let buffer = ''
  const queue = []
  const waiters = []
  stream.setEncoding('utf8')
  stream.on('data', chunk => {
    buffer += chunk
    while (buffer.includes('\n')) {
      const marker = buffer.indexOf('\n')
      const line = buffer.slice(0, marker)
      buffer = buffer.slice(marker + 1)
      if (!line.trim()) continue
      const value = JSON.parse(line)
      const waiter = waiters.shift()
      if (waiter) waiter(value)
      else queue.push(value)
    }
  })
  return () => queue.length > 0 ? Promise.resolve(queue.shift()) : new Promise(resolve => waiters.push(resolve))
}

function websocketFrames(socket) {
  const queue = []
  const waiters = []
  socket.on('message', data => {
    const value = JSON.parse(data.toString())
    const waiter = waiters.shift()
    if (waiter) waiter(value)
    else queue.push(value)
  })
  return () => queue.length > 0 ? Promise.resolve(queue.shift()) : new Promise(resolve => waiters.push(resolve))
}

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'ops-brain-adapter-e2e.'))
const tokenFile = path.join(temporary, 'agent-token')
fs.writeFileSync(tokenFile, 'fixture-token-not-a-secret\n', { mode: 0o600 })

let child
let server
try {
  let connectionResolve
  const connection = new Promise(resolve => { connectionResolve = resolve })
  let connectionCount = 0
  server = new WebSocketServer({
    host: '127.0.0.1',
    port: 0,
    verifyClient: ({ req }) => req.headers.authorization === 'Bearer fixture-token-not-a-secret',
  })
  await new Promise(resolve => server.once('listening', resolve))
  server.on('connection', socket => {
    connectionCount += 1
    connectionResolve(socket)
  })
  const address = server.address()
  assert.equal(typeof address, 'object')

  const environment = {
    ...process.env,
    OPS_BRAIN_LIVE_URL: `ws://127.0.0.1:${address.port}/live`,
    OPS_BRAIN_AGENT_TOKEN_FILE: tokenFile,
    OPS_BRAIN_EXPECTED_AGENT: 'CC-E2E',
    OPS_BRAIN_LIVE_LABEL: 'claude-e2e',
  }
  delete environment.OPS_BRAIN_AGENT_TOKEN
  child = spawn(process.execPath, [path.join(root, 'adapters', 'claude-channel', 'src', 'main.js')], {
    env: environment,
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  const nextMcp = jsonLineReader(child.stdout)
  let stderr = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', chunk => { stderr += chunk })

  await new Promise(resolve => setTimeout(resolve, 150))
  assert.equal(connectionCount, 0, 'adapter registered before the Claude MCP client initialized')

  child.stdin.write(`${JSON.stringify({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'ops-brain-e2e', version: '1.0.0' },
    },
  })}\n`)
  const initialized = await withTimeout(nextMcp(), 'MCP initialize response')
  assert.equal(initialized.id, 1)
  assert.equal(initialized.result.capabilities.experimental['claude/channel'] != null, true)
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`)

  const socket = await withTimeout(connection, 'adapter WebSocket registration')
  const nextFrame = websocketFrames(socket)
  const register = await withTimeout(nextFrame(), 'register frame')
  assert.deepEqual(
    { type: register.type, protocol: register.protocol, adapter: register.adapter, label: register.label },
    { type: 'register', protocol: 1, adapter: 'claude_code', label: 'claude-e2e' },
  )
  const peerId = randomUUID()
  socket.send(JSON.stringify({
    type: 'registered',
    protocol_version: 1,
    peer: { peer_id: peerId, agent_name: 'CC-E2E', adapter: 'claude_code', label: 'claude-e2e' },
  }))

  const messageId = randomUUID()
  socket.send(JSON.stringify({
    type: 'message',
    message: {
      message_id: messageId,
      reply_peer_id: randomUUID(),
      from_agent: 'Codex-E2E',
      body: 'adapter e2e marker',
      trust: 'untrusted_peer_input',
      source_binding: 'connection_bound',
      in_reply_to: null,
    },
  }))
  const notification = await withTimeout(nextMcp(), 'Claude Channel notification')
  assert.equal(notification.method, 'notifications/claude/channel')
  assert.match(notification.params.content, /SECURITY BOUNDARY/)
  assert.match(notification.params.content, /PEER> adapter e2e marker/)
  assert.equal(notification.params.meta.from_agent, 'Codex-E2E')
  const acknowledge = await withTimeout(nextFrame(), 'host acceptance ACK')
  assert.deepEqual(acknowledge, { type: 'acknowledge', message_id: messageId, accepted: true })

  child.stdin.write(`${JSON.stringify({
    jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'list_live_peers', arguments: {} },
  })}\n`)
  const listRequest = await withTimeout(nextFrame(), 'adapter peer-list request')
  assert.equal(listRequest.type, 'list_peers')
  socket.send(JSON.stringify({
    type: 'peers', request_id: listRequest.request_id,
    peers: [{ peer_id: peerId, agent_name: 'CC-E2E', adapter: 'claude_code', label: 'claude-e2e' }],
  }))
  const toolResult = await withTimeout(nextMcp(), 'MCP peer-list tool result')
  assert.equal(toolResult.id, 2)
  assert.equal(toolResult.result.isError, undefined)
  assert.match(toolResult.result.content[0].text, /CC-E2E/)

  assert.equal(stderr.includes('fixture-token-not-a-secret'), false, 'adapter logged the bearer')
  console.log('online adapter end-to-end test passed')
} finally {
  if (child && child.exitCode == null) {
    child.kill('SIGTERM')
    await new Promise(resolve => child.once('exit', resolve))
  }
  if (server) await new Promise(resolve => server.close(resolve))
  fs.rmSync(temporary, { recursive: true, force: true })
}
