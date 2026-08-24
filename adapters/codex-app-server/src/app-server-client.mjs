import { EventEmitter } from 'node:events';
import { spawn } from 'node:child_process';
import readline from 'node:readline';
import WebSocket from 'ws';

class RpcError extends Error {
  constructor(method, error) {
    super(`${method} failed: ${error?.message || 'unknown App Server error'}`);
    this.name = 'RpcError';
    this.code = Number.isSafeInteger(error?.code) ? error.code : undefined;
  }
}

class RpcClient extends EventEmitter {
  constructor({ requestTimeoutMs }) {
    super();
    this.requestTimeoutMs = requestTimeoutMs;
    this.nextId = 1;
    this.pending = new Map();
    this.closed = false;
  }

  request(method, params = undefined) {
    if (this.closed) return Promise.reject(new Error('App Server transport is closed'));
    const id = this.nextId++;
    const message = params === undefined ? { method, id } : { method, id, params };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, this.requestTimeoutMs);
      this.pending.set(id, { method, resolve, reject, timer });
      try {
        this.sendRaw(message);
      } catch (error) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error);
      }
    });
  }

  notify(method, params = {}) {
    this.sendRaw({ method, params });
  }

  receive(message) {
    if (Object.hasOwn(message, 'id') && !message.method) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      clearTimeout(pending.timer);
      this.pending.delete(message.id);
      if (message.error) pending.reject(new RpcError(pending.method, message.error));
      else pending.resolve(message.result);
      return;
    }

    if (message.method && Object.hasOwn(message, 'id')) {
      // Never let peer-originated work grant a host permission. Unsupported
      // server requests fail closed on this adapter connection.
      this.sendRaw({
        id: message.id,
        error: { code: -32601, message: 'ops-brain live adapter cannot authorize host requests' },
      });
      return;
    }

    if (message.method) this.emit('notification', message);
  }

  failPending(error) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }
}

class WebSocketRpcClient extends RpcClient {
  constructor(options) {
    super(options);
    this.url = options.url;
    this.token = options.token;
    this.socket = null;
  }

  async connect() {
    const headers = this.token ? { Authorization: `Bearer ${this.token}` } : undefined;
    const socket = new WebSocket(this.url, {
      headers,
      handshakeTimeout: this.requestTimeoutMs,
      maxPayload: 1024 * 1024,
      perMessageDeflate: false,
    });
    this.socket = socket;
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
    socket.on('message', (data, isBinary) => {
      if (isBinary) return;
      try { this.receive(JSON.parse(data.toString())); }
      catch { this.emit('protocolError', new Error('App Server returned invalid JSON')); }
    });
    socket.once('close', () => {
      this.closed = true;
      this.failPending(new Error('App Server WebSocket closed'));
      this.emit('close');
    });
    socket.on('error', (error) => this.emit('transportError', error));
  }

  sendRaw(message) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      throw new Error('App Server WebSocket is not open');
    }
    this.socket.send(JSON.stringify(message));
  }

  async close() {
    if (!this.socket || this.socket.readyState === WebSocket.CLOSED) return;
    if (this.socket.readyState === WebSocket.CONNECTING) {
      this.socket.terminate();
      return;
    }
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

export class StdioRpcClient extends RpcClient {
  constructor(options) {
    super(options);
    this.codexBin = options.codexBin;
    this.spawnProcess = options.spawnProcess || spawn;
    this.shutdownGraceMs = options.shutdownGraceMs || 1000;
    this.child = null;
    this.lines = null;
  }

  async connect() {
    const child = this.spawnProcess(this.codexBin, ['app-server', '--listen', 'stdio://'], {
      shell: false,
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    this.child = child;
    this.lines = readline.createInterface({ input: child.stdout });
    this.lines.on('line', (line) => {
      try { this.receive(JSON.parse(line)); }
      catch { this.emit('protocolError', new Error('App Server returned invalid JSONL')); }
    });
    child.once('error', (error) => {
      this.closed = true;
      this.failPending(error);
      this.emit('close');
    });
    child.once('exit', (code, signal) => {
      this.closed = true;
      this.failPending(new Error(`App Server exited (${signal || code})`));
      this.emit('close');
    });
  }

  sendRaw(message) {
    if (!this.child?.stdin?.writable) throw new Error('App Server stdin is not writable');
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  async close() {
    this.lines?.close();
    this.lines = null;
    if (!this.child) return;
    if (this.child.stdin?.writable) this.child.stdin.end();
    if (this.child.exitCode !== null || this.child.signalCode !== null) return;
    this.child.kill('SIGTERM');
    if (await waitForExit(this.child, this.shutdownGraceMs)) return;
    this.child.kill('SIGKILL');
    await waitForExit(this.child, this.shutdownGraceMs);
  }
}

function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve(true);
  return new Promise((resolve) => {
    let settled = false;
    const finish = (exited) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.off('exit', onExit);
      resolve(exited);
    };
    const onExit = () => finish(true);
    const timer = setTimeout(() => finish(false), timeoutMs);
    child.once('exit', onExit);
  });
}

export class AppServerClient extends EventEmitter {
  constructor(config) {
    super();
    this.config = config;
    this.stopped = false;
    this.rpcGeneration = 1;
    this.reconnectPromise = null;
    this.rpc = this.#newRpc();
    this.#bindRpc(this.rpc, this.rpcGeneration);
  }

  #newRpc() {
    return this.config.appServerUrl
      ? new WebSocketRpcClient({
          url: this.config.appServerUrl,
          token: this.config.appServerToken,
          requestTimeoutMs: this.config.requestTimeoutMs,
        })
      : new StdioRpcClient({
          codexBin: this.config.codexBin,
          requestTimeoutMs: this.config.requestTimeoutMs,
        });
  }

  #bindRpc(rpc, generation) {
    const current = () => (
      !this.stopped && this.rpc === rpc && this.rpcGeneration === generation
    );
    rpc.on('notification', (event) => { if (current()) this.emit('notification', event); });
    rpc.on('close', () => { if (current()) this.emit('close'); });
    rpc.on('transportError', (error) => { if (current()) this.emit('transportError', error); });
    rpc.on('protocolError', (error) => { if (current()) this.emit('protocolError', error); });
  }

  async connect() {
    if (this.stopped) throw new Error('App Server client is stopped');
    return this.#connectRpc(this.rpc, this.rpcGeneration);
  }

  async #connectRpc(rpc, generation) {
    const assertCurrent = async () => {
      if (!this.stopped && this.rpc === rpc && this.rpcGeneration === generation) return;
      await rpc.close().catch(() => {});
      throw new Error('App Server connection was stopped or superseded');
    };
    await rpc.connect();
    await assertCurrent();
    await rpc.request('initialize', {
      clientInfo: {
        name: 'ops_brain_live',
        title: 'ops-brain live adapter',
        version: '1.0.0',
      },
    });
    await assertCurrent();
    rpc.notify('initialized', {});
  }

  request(method, params) {
    if (this.stopped || !this.rpc) return Promise.reject(new Error('App Server client is stopped'));
    return this.rpc.request(method, params);
  }

  async reconnect() {
    if (!this.config.appServerUrl) {
      throw new Error('cannot reconnect an owned stdio App Server');
    }
    if (this.stopped) throw new Error('App Server client is stopped');
    if (this.reconnectPromise) return this.reconnectPromise;
    const reconnect = this.#replaceRpc();
    this.reconnectPromise = reconnect;
    try {
      await reconnect;
    } finally {
      if (this.reconnectPromise === reconnect) this.reconnectPromise = null;
    }
  }

  async #replaceRpc() {
    const previous = this.rpc;
    const replacement = this.#newRpc();
    const generation = ++this.rpcGeneration;
    this.rpc = replacement;
    this.#bindRpc(replacement, generation);
    await previous?.close().catch(() => {});
    if (this.stopped || this.rpc !== replacement || this.rpcGeneration !== generation) {
      await replacement.close().catch(() => {});
      throw new Error('App Server reconnect was stopped or superseded');
    }
    try {
      await this.#connectRpc(replacement, generation);
    } catch (error) {
      await replacement.close().catch(() => {});
      throw error;
    }
  }

  async close() {
    if (this.stopped) return;
    this.stopped = true;
    this.rpcGeneration += 1;
    const rpc = this.rpc;
    const reconnect = this.reconnectPromise;
    this.rpc = null;
    await rpc?.close().catch(() => {});
    await reconnect?.catch(() => {});
  }
}

export { RpcError };
