#!/usr/bin/env node

import process from 'node:process';
import readline from 'node:readline';
import { CodexLiveBridge } from './bridge.mjs';
import { loadConfig, redactedConfig } from './config.mjs';
import { handleControlCommand } from './control.mjs';

function log(level, message, fields = {}) {
  process.stderr.write(`${JSON.stringify({ level, message, ...fields })}\n`);
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
  bridge.on('warning', (error) => log('warn', error.message));
  bridge.on('deliveryError', ({ messageId, error }) => log('warn', 'live message was not injected', {
    message_id: messageId,
    error_type: error.name,
    delivery_stage: error.deliveryStage || 'unknown',
    error_code: Number.isSafeInteger(error.code) ? error.code : undefined,
  }));

  const controls = readline.createInterface({ input: process.stdin });
  controls.on('line', async (line) => {
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
    controls.close();
    process.stdin.pause();
    await bridge.stop();
  };
  process.once('SIGINT', () => stop('SIGINT'));
  process.once('SIGTERM', () => stop('SIGTERM'));

  try {
    await bridge.start();
  } finally {
    controls.close();
    process.stdin.pause();
  }
}

main().catch((error) => {
  log('error', 'adapter stopped', { error: error.message });
  process.exitCode = 1;
});
