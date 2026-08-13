import { EventEmitter } from 'node:events';
import { spawn } from 'node:child_process';
import readline from 'node:readline';
import WebSocket from 'ws';

class RpcError extends Error {
  constructor(method, error) {
    super(`${method} failed: ${error?.message || 'unknown App Server error'}`);
    this.name = 'RpcError';
    this.code = error?.code;
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
    this.socket = socket;
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
    this.rpc = config.appServerUrl
      ? new WebSocketRpcClient({
          url: config.appServerUrl,
          token: config.appServerToken,
          requestTimeoutMs: config.requestTimeoutMs,
        })
      : new StdioRpcClient({
          codexBin: config.codexBin,
          requestTimeoutMs: config.requestTimeoutMs,
        });
    this.rpc.on('notification', (event) => this.emit('notification', event));
    this.rpc.on('close', () => this.emit('close'));
    this.rpc.on('transportError', (error) => this.emit('transportError', error));
    this.rpc.on('protocolError', (error) => this.emit('protocolError', error));
  }

  async connect() {
    await this.rpc.connect();
    await this.rpc.request('initialize', {
      clientInfo: {
        name: 'ops_brain_live',
        title: 'ops-brain live adapter',
        version: '0.1.0',
      },
    });
    this.rpc.notify('initialized', {});
  }

  request(method, params) {
    return this.rpc.request(method, params);
  }

  close() {
    return this.rpc.close();
  }
}

export { RpcError };
