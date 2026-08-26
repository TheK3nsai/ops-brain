#!/usr/bin/env node

import process from 'node:process';
import readline from 'node:readline';
import { existsSync } from 'node:fs';
import { CodexLiveBridge } from './bridge.mjs';
import { loadConfig, redactedConfig } from './config.mjs';
import { handleControlCommand } from './control.mjs';

function log(level, message, fields = {}) {
  // Timestamped because these logs are the evidence in an attended gate, and
  // each launch writes a separate file with no other ordering between them.
  process.stderr.write(`${JSON.stringify({ ts: new Date().toISOString(), level, message, ...fields })}\n`);
}

function writeControl(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

async function main() {
  const config = loadConfig();
  const bridge = new CodexLiveBridge(config);
  bridge.on('connected', (peer) => log('info', 'live adapter connected', {
    peer_id: peer.peer_id,
    agent_name: peer.agent_name,
    ...redactedConfig(config),
  }));
  bridge.on('disconnected', ({ peer, code, reason, willReconnect }) => log(willReconnect ? 'warn' : 'info', 'live adapter disconnected', {
    peer_id: peer.peer_id,
    agent_name: peer.agent_name,
    close_code: code,
    close_reason: reason || undefined,
    will_reconnect: willReconnect,
  }));
  bridge.on('reconnecting', ({ delayMs }) => log('info', 'live adapter reconnect scheduled', {
    delay_ms: delayMs,
  }));
  bridge.on('fatal', (error) => {
    log('error', error.message, {
      expected_agent: config.expectedAgent,
      retryable: false,
    });
    process.exitCode = 1;
  });
  bridge.on('warning', (error) => log('warn', error.message, {
    delivery_stage: error.deliveryStage || undefined,
    cause: error.cause?.message || undefined,
  }));
  bridge.on('deliveryError', ({ messageId, error }) => log('warn', 'live message was not injected', {
    message_id: messageId,
    error_type: error.name,
    delivery_stage: error.deliveryStage || 'unknown',
    error_code: Number.isSafeInteger(error.code) ? error.code : undefined,
  }));

  // The Windows launcher shares the Codex TUI's console stdin. Only claim the
  // control channel when stdin is a pipe so the adapter cannot consume TUI keys.
  const controls = process.stdin.isTTY
    ? null
    : readline.createInterface({ input: process.stdin });
  let controlInputFailed = false;
  const handleControlInputError = (error) => {
    if (controlInputFailed) return;
    controlInputFailed = true;
    log('warn', 'adapter control input is unavailable; live delivery remains active', {
      error: error?.message || String(error),
    });
    controls?.close();
  };
  if (controls) process.stdin.on('error', handleControlInputError);
  controls?.on('error', handleControlInputError);
  controls?.on('line', async (line) => {
    let command;
    try { command = JSON.parse(line); }
    catch {
      writeControl({ ok: false, error: 'invalid JSON control command' });
      return;
    }
    writeControl(await handleControlCommand(command, bridge));
  });

  let stopping = false;
  const stop = async (signal) => {
    if (stopping) return;
    stopping = true;
    log('info', 'stopping live adapter', { signal });
    controls?.close();
    if (controls) process.stdin.pause();
    await bridge.stop();
  };
  let controlOutputFailed = false;
  const handleControlOutputError = (error) => {
    if (controlOutputFailed) return;
    controlOutputFailed = true;
    log('warn', 'adapter control output is unavailable; stopping live delivery', {
      error: error?.message || String(error),
    });
    void stop('control-output-error').catch((stopError) => {
      log('error', 'adapter graceful stop failed', { error: stopError.message });
      process.exitCode = 1;
    });
  };
  process.stdout.on('error', handleControlOutputError);
  const stopFile = process.env.OPS_BRAIN_LIVE_STOP_FILE;
  const stopPoll = stopFile ? setInterval(() => {
    if (!existsSync(stopFile)) return;
    void stop('launcher-stop-file').catch((error) => {
      log('error', 'adapter graceful stop failed', { error: error.message });
      process.exitCode = 1;
    });
  }, 100) : null;
  stopPoll?.unref();
  process.once('SIGINT', () => stop('SIGINT'));
  process.once('SIGTERM', () => stop('SIGTERM'));

  try {
    await bridge.start();
  } finally {
    if (stopPoll) clearInterval(stopPoll);
    if (controls) process.stdin.off('error', handleControlInputError);
    process.stdout.off('error', handleControlOutputError);
    controls?.close();
    if (controls) process.stdin.pause();
  }
}

main().catch((error) => {
  log('error', 'adapter stopped', { error: error.message });
  process.exitCode = 1;
});
