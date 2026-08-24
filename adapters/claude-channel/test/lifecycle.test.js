import assert from 'node:assert/strict'
import test from 'node:test'
import { bindLiveLifecycle } from '../src/lifecycle.js'

test('registers with ops-brain only after the Claude MCP client initializes', () => {
  const mcp = {}
  let starts = 0
  let stops = 0
  const stop = bindLiveLifecycle(mcp, {
    start: () => { starts += 1 },
    stop: () => { stops += 1 },
  })

  assert.equal(starts, 0)
  mcp.oninitialized()
  assert.equal(starts, 1)
  mcp.oninitialized()
  assert.equal(starts, 1)
  stop()
  assert.equal(stops, 1)
})

test('preserves an existing initialization handler and does not stop a never-started client', async () => {
  let previousCalls = 0
  let starts = 0
  let stops = 0
  const warnings = []
  const mcp = { oninitialized: () => { previousCalls += 1 } }
  const stop = bindLiveLifecycle(mcp, {
    start: () => { starts += 1 },
    stop: () => { stops += 1 },
  }, { warningMs: 5, warn: message => warnings.push(message) })

  await new Promise(resolve => setTimeout(resolve, 10))
  assert.equal(warnings.length, 1)
  stop()
  assert.equal(stops, 0)

  const secondStop = bindLiveLifecycle(mcp, {
    start: () => { starts += 1 },
    stop: () => { stops += 1 },
  }, { warningMs: 100, warn: () => {} })
  mcp.oninitialized()
  assert.equal(previousCalls, 1)
  assert.equal(starts, 1)
  secondStop()
  assert.equal(stops, 1)
})

test('stopping before the client ever initializes does not start the connection', () => {
  const mcp = {}
  let starts = 0
  let stops = 0
  const stop = bindLiveLifecycle(mcp, {
    start: () => { starts += 1 },
    stop: () => { stops += 1 },
  })

  stop()
  assert.equal(starts, 0)
  assert.equal(stops, 0)
})
