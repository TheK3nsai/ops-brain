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

test('treats server and legacy unconfirmed delivery results as errors', async t => {
  FakeWebSocket.instances.length = 0
  FakeWebSocket.sendOutcome = 'delivery_unconfirmed'
  t.after(() => { FakeWebSocket.sendOutcome = 'host_accepted' })
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

  await assert.rejects(
    client.sendMessage({ toPeerId: CODEX_PEER, body: 'ambiguous' }),
    /delivery was not confirmed; re-list live peers.*handoff/,
  )

  FakeWebSocket.sendOutcome = 'routed'
  await assert.rejects(
    client.sendMessage({ toPeerId: CODEX_PEER, body: 'legacy ambiguous' }),
    /delivery was not confirmed; re-list live peers.*handoff/,
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
  const fatal = once(client, 'fatal')
  client.start()
  const [error] = await fatal
  assert.equal(client.ready, false)
  assert.match(error.message, /bound identity does not match/)
  assert.match(error.message, /CC-Cloud/)
  assert.equal(client.fatal, error)
  assert.equal(diagnostics.length, 1)
  assert.match(diagnostics[0], /bound identity does not match/)
})

test('does not reconnect after an identity mismatch', async t => {
  FakeWebSocket.instances.length = 0
  const client = new LiveClient(
    {
      url: 'ws://127.0.0.1:3000/live',
      token: 'wrong-sibling-token',
      label: 'claude-test',
      expectedAgent: 'CC-Cloud',
    },
    { WebSocketImpl: FakeWebSocket, logger: () => {} },
  )
  t.after(() => client.stop())
  client.start()
  await once(client, 'fatal')
  // The shortest reconnect backoff is 500ms; a retry would open a second
  // socket well inside this window. Retrying would re-send the bearer on a
  // configuration error that cannot resolve itself.
  await new Promise(resolve => setTimeout(resolve, 900))
  assert.equal(FakeWebSocket.instances.length, 1)
})

test('reports the fatal reason instead of a generic offline error', async t => {
  FakeWebSocket.instances.length = 0
  const client = new LiveClient(
    {
      url: 'ws://127.0.0.1:3000/live',
      token: 'wrong-sibling-token',
      label: 'claude-test',
      expectedAgent: 'CC-Cloud',
    },
    { WebSocketImpl: FakeWebSocket, logger: () => {} },
  )
  t.after(() => client.stop())
  client.start()
  await once(client, 'fatal')
  // Claude Code discards this adapter's stderr, so the tool error is the
  // operator's only in-session signal; "offline" would misdirect them.
  await assert.rejects(() => client.listPeers(), /bound identity does not match/)
  await assert.rejects(() => client.waitUntilReady(50), /bound identity does not match/)
})

class FakeWebSocket extends EventEmitter {
  static OPEN = 1
  static instances = []
  static sendOutcome = 'host_accepted'

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
      if (FakeWebSocket.sendOutcome === 'delivery_unconfirmed') {
        this.serverSend({
          type: 'error',
          request_id: frame.request_id,
          code: 'delivery_unconfirmed',
          message: 'untrusted server detail must not be rendered',
        })
      } else {
        this.serverSend({
          type: 'send_result',
          request_id: frame.request_id,
          receipt: {
            message_id: MESSAGE_ID,
            status: FakeWebSocket.sendOutcome,
            detail: 'test receipt',
          },
        })
      }
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
