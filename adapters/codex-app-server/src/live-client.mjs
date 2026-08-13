import { EventEmitter } from 'node:events';
import { randomUUID } from 'node:crypto';
import WebSocket from 'ws';
import {
  assertExactKeys,
  assertMessageBody,
  assertOptionalUuid,
  assertUuid,
  isRecord,
  validateErrorFrame,
  validateIncomingMessage,
  validatePeersFrame,
  validateRegisteredFrame,
  validateSendResultFrame,
} from './protocol.mjs';

const MAX_PENDING_REQUESTS = 16;

export class LiveClient extends EventEmitter {
  constructor(config) {
    super();
    this.config = config;
    this.socket = null;
    this.peer = null;
    this.pending = new Map();
    this.registerTimer = null;
  }

  async connect() {
    const socket = new WebSocket(this.config.liveUrl, {
      headers: { Authorization: `Bearer ${this.config.agentToken}` },
      handshakeTimeout: this.config.requestTimeoutMs,
      maxPayload: 16 * 1024,
      perMessageDeflate: false,
    });
    this.socket = socket;
    socket.on('message', (data, isBinary) => this.#receive(data, isBinary));
    socket.on('error', (error) => this.emit('transportError', error));
    socket.once('close', () => {
      clearTimeout(this.registerTimer);
      this.peer = null;
      for (const pending of this.pending.values()) {
        clearTimeout(pending.timer);
        pending.reject(new Error('ops-brain live connection closed'));
      }
      this.pending.clear();
      this.emit('close');
    });

    await new Promise((resolve, reject) => {
      const onOpen = () => { cleanup(); resolve(); };
      const onError = (error) => { cleanup(); reject(error); };
      const cleanup = () => {
        socket.off('open', onOpen);
        socket.off('error', onError);
      };
      socket.once('open', onOpen);
      socket.once('error', onError);
    });

    const registration = new Promise((resolve, reject) => {
      const onRegistered = () => { cleanup(); resolve(); };
      const onClose = () => { cleanup(); reject(new Error('live connection closed before registration')); };
      const cleanup = () => {
        clearTimeout(this.registerTimer);
        this.off('registered', onRegistered);
        this.off('close', onClose);
      };
      this.once('registered', onRegistered);
      this.once('close', onClose);
      this.registerTimer = setTimeout(() => {
        cleanup();
        reject(new Error('ops-brain live registration timed out'));
        socket.close(1008, 'registration timeout');
      }, this.config.requestTimeoutMs);
    });
    socket.send(JSON.stringify({
      type: 'register',
      protocol: 1,
      adapter: 'codex',
      label: this.config.label,
    }));
    await registration;
    return this.peer;
  }

  #receive(data, isBinary) {
    if (isBinary) {
      this.#protocolFailure(new Error('ops-brain returned a binary frame'));
      return;
    }
    let frame;
    try { frame = JSON.parse(data.toString()); }
    catch {
      this.#protocolFailure(new Error('ops-brain returned invalid JSON'));
      return;
    }

    if (!isRecord(frame) || typeof frame.type !== 'string') {
      this.#protocolFailure(new Error('ops-brain returned a non-object frame'));
      return;
    }

    if (frame.type === 'registered') {
      try {
        if (this.peer) throw new Error('ops-brain registered this connection more than once');
        const candidate = validateRegisteredFrame(frame, this.config.label);
        if (candidate.agent_name.toLowerCase() !== this.config.expectedAgent.toLowerCase()) {
          throw new Error('ops-brain bound identity does not match OPS_BRAIN_EXPECTED_AGENT');
        }
        this.peer = candidate;
      } catch (error) {
        this.#protocolFailure(error);
        return;
      }
      this.emit('registered', this.peer);
      return;
    }
    if (frame.type === 'message') {
      try {
        assertExactKeys(frame, ['type', 'message'], [], 'message frame');
        validateIncomingMessage(frame.message);
      } catch (error) {
        this.#protocolFailure(error);
        return;
      }
      this.emit('message', frame.message);
      return;
    }
    if (frame.type === 'peers' || frame.type === 'send_result' || frame.type === 'error') {
      try {
        if (frame.type === 'peers') validatePeersFrame(frame);
        else if (frame.type === 'send_result') validateSendResultFrame(frame);
        else validateErrorFrame(frame);
      } catch (error) {
        this.#protocolFailure(error);
        return;
      }
      const pending = frame.request_id ? this.pending.get(frame.request_id) : null;
      if (!pending) {
        if (frame.type === 'error') this.emit('serverError', frame);
        else this.#protocolFailure(new Error(`unsolicited ${frame.type} frame`));
        return;
      }
      const expectedType = pending.kind === 'list_peers' ? 'peers' : 'send_result';
      if (frame.type !== 'error' && frame.type !== expectedType) {
        this.#protocolFailure(new Error(`expected ${expectedType}, received ${frame.type}`));
        return;
      }
      clearTimeout(pending.timer);
      this.pending.delete(frame.request_id);
      if (frame.type === 'error') pending.reject(new Error(`${frame.code}: ${frame.message}`));
      else pending.resolve(frame.type === 'peers' ? frame.peers : frame.receipt);
      return;
    }
    this.#protocolFailure(new Error(`unsupported ops-brain frame type: ${frame.type}`));
  }

  #request(frame) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN || !this.peer) {
      return Promise.reject(new Error('ops-brain live connection is not registered'));
    }
    const requestId = frame.request_id || randomUUID();
    try { assertUuid(requestId, 'request_id'); }
    catch (error) { return Promise.reject(error); }
    if (this.pending.size >= MAX_PENDING_REQUESTS) {
      return Promise.reject(new Error(`too many live requests in flight (max ${MAX_PENDING_REQUESTS})`));
    }
    if (this.pending.has(requestId)) {
      return Promise.reject(new Error('duplicate live request_id is already in flight'));
    }
    frame.request_id = requestId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error(`${frame.type} timed out`));
      }, this.config.requestTimeoutMs);
      this.pending.set(requestId, { kind: frame.type, resolve, reject, timer });
      this.socket.send(JSON.stringify(frame), (error) => {
        if (!error) return;
        clearTimeout(timer);
        this.pending.delete(requestId);
        reject(error);
      });
    });
  }

  listPeers() {
    return this.#request({ type: 'list_peers', request_id: randomUUID() });
  }

  sendMessage({ toPeerId, body, inReplyTo = null, requestId = randomUUID() }) {
    try {
      assertUuid(toPeerId, 'toPeerId');
      assertMessageBody(body, 'body');
      assertOptionalUuid(inReplyTo, 'inReplyTo');
      assertUuid(requestId, 'requestId');
    } catch (error) {
      return Promise.reject(error);
    }
    return this.#request({
      type: 'send_message',
      request_id: requestId,
      to_peer_id: toPeerId,
      body,
      in_reply_to: inReplyTo,
    });
  }

  acknowledge(messageId, accepted) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN || !this.peer) return false;
    try {
      assertUuid(messageId, 'messageId');
      if (typeof accepted !== 'boolean') throw new Error('accepted must be boolean');
    } catch (error) {
      this.emit('protocolError', error);
      return false;
    }
    this.socket.send(JSON.stringify({
      type: 'acknowledge',
      message_id: messageId,
      accepted,
    }));
    return true;
  }

  isRegisteredPeer(peerId) {
    return this.socket?.readyState === WebSocket.OPEN && this.peer?.peer_id === peerId;
  }

  #protocolFailure(error) {
    this.peer = null;
    this.emit('protocolError', error);
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.close(1002, 'invalid live protocol frame');
    } else if (this.socket?.readyState === WebSocket.CONNECTING) {
      this.socket.terminate();
    }
  }

  async close() {
    if (!this.socket || this.socket.readyState === WebSocket.CLOSED) return;
    await new Promise((resolve) => {
      this.socket.once('close', resolve);
      this.socket.close(1000, 'adapter stopping');
      setTimeout(() => {
        if (this.socket?.readyState !== WebSocket.CLOSED) this.socket?.terminate();
        resolve();
      }, 1000).unref();
    });
  }
}
