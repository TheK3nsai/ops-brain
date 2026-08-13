import assert from 'node:assert/strict'
import test from 'node:test'
import { EventEmitter, once } from 'node:events'
import { LiveClient } from '../src/live-client.js'

const CLAUDE_PEER = '019cd123-1234-7123-8123-123456789abc'
const CODEX_PEER = '019cd123-1234-7123-8123-123456789abd'
const MESSAGE_ID = '019cd123-1234-7123-8123-123456789abe'

test('authenticates, registers, lists, sends, receives, and acknowledges', async t => {
  FakeWebSocket.instances.length = 0
  const client = new LiveClient(
    {
      url: 'ws://127.0.0.1:3000/live',
      token: 'test-agent-token',
      label: 'claude-test',
      expectedAgent: 'CC-Stealth',
    },
    { WebSocketImpl: FakeWebSocket, logger: () => {} },
  )
  t.after(() => client.stop())
  client.start()
  await client.waitUntilReady()

  const socket = FakeWebSocket.instances[0]
  assert.equal(socket.url, 'ws://127.0.0.1:3000/live')
  assert.equal(socket.options.headers.Authorization, 'Bearer test-agent-token')
  assert.equal(socket.options.headers.Origin, undefined)
  assert.equal(socket.sent[0].type, 'register')
  assert.equal(socket.sent[0].adapter, 'claude_code')

  const listed = await client.listPeers()
  assert.equal(listed.peers[0].peer_id, CODEX_PEER)
  const sent = await client.sendMessage({ toPeerId: CODEX_PEER, body: 'hello', inReplyTo: null })
  assert.equal(sent.receipt.status, 'host_accepted')

  const incoming = once(client, 'message')
  socket.serverSend({
    type: 'message',
    message: {
      message_id: MESSAGE_ID,
      reply_peer_id: CODEX_PEER,
      from_agent: 'Codex-Stealth',
      body: 'hello from Codex',
      trust: 'untrusted_peer_input',
      source_binding: 'connection_bound',
    },
  })
  const [message] = await incoming
  assert.equal(message.body, 'hello from Codex')
  assert.equal(client.acknowledge(MESSAGE_ID, true), true)
  const ack = socket.sent.find(frame => frame.type === 'acknowledge')
  assert.deepEqual(ack, { type: 'acknowledge', message_id: MESSAGE_ID, accepted: true })
})

test('does not queue requests while disconnected', async () => {
  const client = new LiveClient(
    { url: 'ws://127.0.0.1:9/live', token: 'x', label: 'offline', expectedAgent: 'CC-Stealth' },
    { WebSocketImpl: NeverOpenedWebSocket, logger: () => {} },
  )
  await assert.rejects(client.listPeers(), /offline/)
  await assert.rejects(
    client.sendMessage({ toPeerId: CODEX_PEER, body: 'not queued' }),
    /offline/,
  )
})

test('fails closed when the token is bound to an unexpected identity', async t => {
  FakeWebSocket.instances.length = 0
  const diagnostics = []
  const client = new LiveClient(
    {
      url: 'ws://127.0.0.1:3000/live',
      token: 'wrong-sibling-token',
      label: 'claude-test',
      expectedAgent: 'CC-Cloud',
    },
    { WebSocketImpl: FakeWebSocket, logger: message => diagnostics.push(message) },
  )
  t.after(() => client.stop())
  const disconnected = once(client, 'disconnected')
  client.start()
  await disconnected
  assert.equal(client.ready, false)
  assert.deepEqual(diagnostics, ['ops-brain bound identity does not match OPS_BRAIN_EXPECTED_AGENT'])
})

class FakeWebSocket extends EventEmitter {
  static OPEN = 1
  static instances = []

  constructor(url, options) {
    super()
    this.url = url
    this.options = options
    this.readyState = 0
    this.sent = []
    FakeWebSocket.instances.push(this)
    setImmediate(() => {
      this.readyState = FakeWebSocket.OPEN
      this.emit('open')
    })
  }

  send(raw) {
    const frame = JSON.parse(raw)
    this.sent.push(frame)
    if (frame.type === 'register') {
      this.serverSend({
        type: 'registered',
        protocol_version: 1,
        peer: {
          peer_id: CLAUDE_PEER,
          agent_name: 'CC-Stealth',
          adapter: 'claude_code',
          label: 'claude-test',
          metadata_trust: 'self_reported',
        },
      })
    } else if (frame.type === 'list_peers') {
      this.serverSend({
        type: 'peers',
        request_id: frame.request_id,
        peers: [{ peer_id: CODEX_PEER, agent_name: 'Codex-Stealth', adapter: 'codex', label: 'codex-test' }],
      })
    } else if (frame.type === 'send_message') {
      this.serverSend({
        type: 'send_result',
        request_id: frame.request_id,
        receipt: { message_id: MESSAGE_ID, status: 'host_accepted', detail: 'accepted' },
      })
    }
  }

  serverSend(frame) {
    setImmediate(() => this.emit('message', Buffer.from(JSON.stringify(frame)), false))
  }

  close() {
    this.readyState = 3
    setImmediate(() => this.emit('close'))
  }
}

class NeverOpenedWebSocket extends EventEmitter {
  static OPEN = 1
  constructor() {
    super()
    this.readyState = 0
  }
  close() {}
}
