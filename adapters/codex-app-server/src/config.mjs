import process from "node:process";
import { closeSync, constants, fstatSync, openSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const LABEL_RE = /^[A-Za-z0-9._-]+$/;

function required(name, env) {
  const value = env[name]?.trim();
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function websocketUrl(name, raw, { loopbackPlaintextOnly = true, exactPath = null } = {}) {
  let url;
  try {
    url = new URL(raw);
  } catch {
    throw new Error(`${name} must be a valid ws:// or wss:// URL`);
  }
  if (!['ws:', 'wss:'].includes(url.protocol)) {
    throw new Error(`${name} must use ws:// or wss://`);
  }
  if (url.username || url.password || url.search || url.hash) {
    throw new Error(`${name} must not contain credentials, a query string, or a fragment`);
  }
  if (loopbackPlaintextOnly && url.protocol === 'ws:' && !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)) {
    throw new Error(`${name} may use plaintext ws:// only on loopback`);
  }
  if (exactPath !== null && url.pathname !== exactPath) {
    throw new Error(`${name} path must be exactly ${exactPath}`);
  }
  return url.toString();
}

export function loadConfig(env = process.env) {
  const expectedAgent = required('OPS_BRAIN_EXPECTED_AGENT', env);
  if (Buffer.byteLength(expectedAgent) > 80 || !LABEL_RE.test(expectedAgent)) {
    throw new Error('OPS_BRAIN_EXPECTED_AGENT must be 1-80 ASCII letters, digits, dot, underscore, or dash');
  }
  const label = (env.OPS_BRAIN_CODEX_LABEL || 'codex-live').trim();
  if (!label || Buffer.byteLength(label) > 80 || !LABEL_RE.test(label)) {
    throw new Error('OPS_BRAIN_CODEX_LABEL must be 1-80 ASCII letters, digits, dot, underscore, or dash');
  }

  const liveUrl = websocketUrl(
    'OPS_BRAIN_LIVE_URL',
    required('OPS_BRAIN_LIVE_URL', env),
    { exactPath: '/live' },
  );
  const appServerUrl = env.OPS_BRAIN_CODEX_APP_SERVER_URL?.trim()
    ? websocketUrl(
        'OPS_BRAIN_CODEX_APP_SERVER_URL',
        env.OPS_BRAIN_CODEX_APP_SERVER_URL.trim(),
        { loopbackPlaintextOnly: true },
      )
    : null;

  const requestTimeoutMs = Number(env.OPS_BRAIN_CODEX_REQUEST_TIMEOUT_MS || 5000);
  if (!Number.isSafeInteger(requestTimeoutMs) || requestTimeoutMs < 500 || requestTimeoutMs > 30000) {
    throw new Error('OPS_BRAIN_CODEX_REQUEST_TIMEOUT_MS must be an integer from 500 to 30000');
  }

  return Object.freeze({
    liveUrl,
    agentToken: loadAgentToken(env),
    expectedAgent,
    label,
    threadId: env.OPS_BRAIN_CODEX_THREAD_ID?.trim() || null,
    appServerUrl,
    appServerToken: optionalSingleLineToken('OPS_BRAIN_CODEX_APP_SERVER_TOKEN', env),
    codexBin: env.OPS_BRAIN_CODEX_BIN?.trim() || 'codex',
    requestTimeoutMs,
    reconnectMinMs: 250,
    reconnectMaxMs: 10000,
    deliveryQueueCapacity: 32,
  });
}

function loadAgentToken(env) {
  const inline = env.OPS_BRAIN_AGENT_TOKEN;
  const file = env.OPS_BRAIN_AGENT_TOKEN_FILE?.trim();
  const helper = env.OPS_BRAIN_AGENT_TOKEN_HELPER_JSON?.trim();
  if ([inline, file, helper].filter(Boolean).length > 1) {
    throw new Error('set only one agent token source: inline, file, or helper');
  }
  if (helper) return loadTokenFromHelper(helper);
  if (!file) return singleLineToken('OPS_BRAIN_AGENT_TOKEN', env);

  let descriptor;
  try {
    const flags = constants.O_RDONLY | (process.platform === 'win32' ? 0 : constants.O_NOFOLLOW);
    descriptor = openSync(file, flags);
    const stat = fstatSync(descriptor);
    if (!stat.isFile() || (process.platform !== 'win32' && (stat.mode & 0o077) !== 0)) {
      throw new Error('unsafe token file');
    }
    const value = readFileSync(descriptor, 'utf8').trim();
    return singleLineToken('OPS_BRAIN_AGENT_TOKEN_FILE', { OPS_BRAIN_AGENT_TOKEN_FILE: value });
  } catch {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_FILE must be a protected regular file');
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function loadTokenFromHelper(raw) {
  let command;
  try { command = JSON.parse(raw); }
  catch { throw new Error('OPS_BRAIN_AGENT_TOKEN_HELPER_JSON must be JSON'); }
  if (
    !Array.isArray(command) || command.length < 1 || command.length > 32
    || command.some(value => typeof value !== 'string' || !value || /[\r\n\0]/.test(value))
  ) {
    throw new Error('OPS_BRAIN_AGENT_TOKEN_HELPER_JSON must be an array of safe command strings');
  }
  try {
    return singleLineToken('agent token helper output', {
      'agent token helper output': execFileSync(command[0], command.slice(1), {
        encoding: 'utf8', input: '', timeout: 5_000, maxBuffer: 16_384,
        windowsHide: true,
      }).trim(),
    });
  } catch {
    throw new Error('agent token helper failed');
  }
}

function singleLineToken(name, env) {
  const value = env[name];
  if (typeof value !== 'string' || !value || value.trim() !== value) {
    throw new Error(`${name} is required and must not have surrounding whitespace`);
  }
  if (/\r|\n/.test(value)) throw new Error(`${name} must be a single line`);
  return value;
}

function optionalSingleLineToken(name, env) {
  const value = env[name];
  if (value === undefined || value === '') return null;
  if (typeof value !== 'string' || value.trim() !== value || /\r|\n/.test(value)) {
    throw new Error(`${name} must be a single line without surrounding whitespace`);
  }
  return value;
}

export function redactedConfig(config) {
  return {
    liveUrl: config.liveUrl,
    label: config.label,
    expectedAgent: config.expectedAgent,
    threadId: config.threadId,
    appServer: config.appServerUrl || 'spawned stdio',
    requestTimeoutMs: config.requestTimeoutMs,
  };
}
