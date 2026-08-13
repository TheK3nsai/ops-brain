import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import WebSocket from 'ws'

const PROTOCOL_VERSION = 1
const REQUEST_TIMEOUT_MS = 5_000
const REGISTER_TIMEOUT_MS = 6_000
const MAX_PENDING_REQUESTS = 16
const MAX_PAYLOAD_BYTES = 16 * 1024
const RECONNECT_MIN_MS = 500
const RECONNECT_MAX_MS = 30_000

export class LiveClient extends EventEmitter {
  #config
  #WebSocket
  #logger
  #socket = null
  #peer = null
  #pending = new Map()
  #stopped = true
  #reconnectTimer = null
  #registerTimer = null
  #attempt = 0

  constructor(config, { WebSocketImpl = WebSocket, logger = defaultLogger } = {}) {
    super()
    this.#config = config
    this.#WebSocket = WebSocketImpl
    this.#logger = logger
  }

  get peer() {
    return this.#peer
  }

  get ready() {
    return this.#socket?.readyState === this.#WebSocket.OPEN && this.#peer !== null
  }

  start() {
    if (!this.#stopped) return
    this.#stopped = false
    this.#connect()
  }

  stop() {
    this.#stopped = true
    clearTimeout(this.#reconnectTimer)
    clearTimeout(this.#registerTimer)
    this.#reconnectTimer = null
    this.#registerTimer = null
    this.#rejectPending('ops-brain live connection stopped')
    this.#socket?.close(1000, 'adapter stopping')
    this.#socket = null
    this.#peer = null
  }

  async waitUntilReady(timeoutMs = 5_000) {
    if (this.ready) return this.#peer
    return new Promise((resolve, reject) => {
      const onReady = peer => {
        cleanup()
        resolve(peer)
      }
      const timer = setTimeout(() => {
        cleanup()
        reject(new Error('ops-brain live connection is not ready'))
      }, timeoutMs)
      const cleanup = () => {
        clearTimeout(timer)
        this.off('ready', onReady)
      }
      this.on('ready', onReady)
    })
  }

  listPeers() {
    return this.#request('list_peers', {})
  }

  sendMessage({ toPeerId, body, inReplyTo = null }) {
    return this.#request('send_message', {
      to_peer_id: toPeerId,
      body,
      in_reply_to: inReplyTo,
    })
  }

  acknowledge(messageId, accepted) {
    if (!this.ready) return false
    return this.#send({ type: 'acknowledge', message_id: messageId, accepted })
  }

  #connect() {
    if (this.#stopped || this.#socket) return
    let socket
    try {
      socket = new this.#WebSocket(this.#config.url, {
        headers: { Authorization: `Bearer ${this.#config.token}` },
        maxPayload: MAX_PAYLOAD_BYTES,
        handshakeTimeout: 5_000,
        perMessageDeflate: false,
      })
    } catch {
      this.#scheduleReconnect()
      return
    }
    this.#socket = socket

    socket.on('open', () => {
      this.#send({
        type: 'register',
        protocol: PROTOCOL_VERSION,
        adapter: 'claude_code',
        label: this.#config.label,
      })
      this.#registerTimer = setTimeout(() => {
        if (!this.#peer) socket.close(1008, 'registration timeout')
      }, REGISTER_TIMEOUT_MS)
    })
    socket.on('message', (data, isBinary) => {
      if (isBinary || Buffer.byteLength(data) > MAX_PAYLOAD_BYTES) {
        socket.close(1009, 'text frames only')
        return
      }
      this.#handleFrame(data.toString())
    })
    socket.on('error', () => {
      this.#logger('ops-brain live WebSocket error')
    })
    socket.on('close', () => {
      if (this.#socket !== socket) return
      clearTimeout(this.#registerTimer)
      this.#registerTimer = null
      this.#socket = null
      this.#peer = null
      this.#rejectPending('ops-brain live peer disconnected; use a handoff for offline delivery')
      this.emit('disconnected')
      this.#scheduleReconnect()
    })
  }

  #scheduleReconnect() {
    if (this.#stopped || this.#reconnectTimer) return
    const base = Math.min(RECONNECT_MIN_MS * 2 ** this.#attempt, RECONNECT_MAX_MS)
    const delay = Math.round(base * (0.8 + Math.random() * 0.4))
    this.#attempt += 1
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = null
      this.#connect()
    }, delay)
  }

  #handleFrame(raw) {
    let frame
    try {
      frame = JSON.parse(raw)
    } catch {
      this.#socket?.close(1007, 'invalid JSON')
      return
    }
    if (!frame || typeof frame !== 'object' || typeof frame.type !== 'string') return

    if (frame.type === 'registered') {
      if (frame.protocol_version !== PROTOCOL_VERSION || !validPeer(frame.peer)) {
        this.#socket?.close(1008, 'invalid registration')
        return
      }
      if (frame.peer.agent_name.toLowerCase() !== this.#config.expectedAgent.toLowerCase()) {
        this.#logger('ops-brain bound identity does not match OPS_BRAIN_EXPECTED_AGENT')
        this.#socket?.close(1008, 'unexpected bound identity')
        return
      }
      clearTimeout(this.#registerTimer)
      this.#registerTimer = null
      this.#peer = Object.freeze({ ...frame.peer })
      this.#attempt = 0
      this.emit('ready', this.#peer)
      return
    }
    if (!this.#peer) return

    if (frame.type === 'message') {
      this.emit('message', frame.message)
      return
    }
    if (frame.type === 'peers' || frame.type === 'send_result' || frame.type === 'error') {
      const pending = typeof frame.request_id === 'string' ? this.#pending.get(frame.request_id) : null
      if (!pending) return
      clearTimeout(pending.timer)
      this.#pending.delete(frame.request_id)
      if (frame.type === 'error') {
        pending.reject(new Error(`ops-brain live request failed (${safeCode(frame.code)})`))
      } else if (frame.type === 'peers') {
        pending.resolve({ peers: Array.isArray(frame.peers) ? frame.peers : [] })
      } else {
        pending.resolve({ receipt: frame.receipt })
      }
    }
  }

  #request(type, fields) {
    if (!this.ready) {
      return Promise.reject(
        new Error('ops-brain live peer is offline; use a handoff for durable delivery'),
      )
    }
    if (this.#pending.size >= MAX_PENDING_REQUESTS) {
      return Promise.reject(new Error('too many ops-brain live requests are in flight'))
    }
    const requestId = randomUUID()
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(requestId)
        reject(new Error('ops-brain live request timed out; delivery was not confirmed'))
      }, REQUEST_TIMEOUT_MS)
      this.#pending.set(requestId, { resolve, reject, timer })
      if (!this.#send({ type, request_id: requestId, ...fields })) {
        clearTimeout(timer)
        this.#pending.delete(requestId)
        reject(new Error('ops-brain live connection closed before the request was sent'))
      }
    })
  }

  #send(frame) {
    if (this.#socket?.readyState !== this.#WebSocket.OPEN) return false
    try {
      this.#socket.send(JSON.stringify(frame))
      return true
    } catch {
      return false
    }
  }

  #rejectPending(message) {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer)
      pending.reject(new Error(message))
    }
    this.#pending.clear()
  }
}

function validPeer(peer) {
  return (
    peer &&
    typeof peer === 'object' &&
    isUuid(peer.peer_id) &&
    typeof peer.agent_name === 'string' &&
    peer.adapter === 'claude_code' &&
    typeof peer.label === 'string'
  )
}

function isUuid(value) {
  return typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)
}

function safeCode(value) {
  return typeof value === 'string' && /^[a-z_]{1,40}$/.test(value) ? value : 'unknown'
}

function defaultLogger(message) {
  process.stderr.write(`${message}\n`)
}
