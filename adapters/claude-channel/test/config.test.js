import assert from 'node:assert/strict'
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { loadConfig } from '../src/config.js'

test('loads a secure live URL and token from environment only', () => {
  const config = loadConfig({
    OPS_BRAIN_LIVE_URL: 'wss://ops.example.com/live',
    OPS_BRAIN_AGENT_TOKEN: 'agent-token',
    OPS_BRAIN_LIVE_LABEL: 'claude.one',
  })
  assert.equal(config.url, 'wss://ops.example.com/live')
  assert.equal(config.token, 'agent-token')
  assert.equal(config.label, 'claude.one')
})

test('permits plaintext WebSocket only on loopback', () => {
  assert.equal(
    loadConfig({ OPS_BRAIN_LIVE_URL: 'ws://127.0.0.1:3000/live', OPS_BRAIN_AGENT_TOKEN: 'x' }).url,
    'ws://127.0.0.1:3000/live',
  )
  assert.throws(
    () => loadConfig({ OPS_BRAIN_LIVE_URL: 'ws://ops.example.com/live', OPS_BRAIN_AGENT_TOKEN: 'x' }),
    /must use wss/,
  )
})

test('rejects credentials, query strings, wrong paths, and unsafe labels', () => {
  for (const url of [
    'wss://user:pass@ops.example.com/live',
    'wss://ops.example.com/live?token=nope',
    'wss://ops.example.com/mcp',
  ]) {
    assert.throws(() => loadConfig({ OPS_BRAIN_LIVE_URL: url, OPS_BRAIN_AGENT_TOKEN: 'x' }))
  }
  assert.throws(() =>
    loadConfig({
      OPS_BRAIN_LIVE_URL: 'wss://ops.example.com/live',
      OPS_BRAIN_AGENT_TOKEN: 'x',
      OPS_BRAIN_LIVE_LABEL: 'label with spaces',
    }),
  )
})

test('loads a protected token file without exposing it through the parent environment', t => {
  const dir = mkdtempSync(join(tmpdir(), 'ops-brain-claude-token-'))
  const file = join(dir, 'token')
  t.after(() => rmSync(dir, { recursive: true }))
  writeFileSync(file, 'file-agent-token\n', { mode: 0o600 })

  const config = loadConfig({
    OPS_BRAIN_LIVE_URL: 'wss://ops.example.com/live',
    OPS_BRAIN_AGENT_TOKEN_FILE: file,
  })
  assert.equal(config.token, 'file-agent-token')

  assert.throws(() => loadConfig({
    OPS_BRAIN_LIVE_URL: 'wss://ops.example.com/live',
    OPS_BRAIN_AGENT_TOKEN: 'inline',
    OPS_BRAIN_AGENT_TOKEN_FILE: file,
  }), /only one/)

  chmodSync(file, 0o644)
  assert.throws(() => loadConfig({
    OPS_BRAIN_LIVE_URL: 'wss://ops.example.com/live',
    OPS_BRAIN_AGENT_TOKEN_FILE: file,
  }), /inaccessible to group\/other/)
})
