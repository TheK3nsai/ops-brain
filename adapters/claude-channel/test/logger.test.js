import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createLogger } from '../src/logger.js'

function scratch() {
  return mkdtempSync(join(tmpdir(), 'ops-brain-logger-'))
}

function readOnlyLog(dir) {
  const files = readdirSync(dir).filter(name => name.startsWith('claude-adapter.'))
  assert.equal(files.length, 1)
  return { name: files[0], body: readFileSync(join(dir, files[0]), 'utf8') }
}

test('records structured lines in the state directory', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const logger = createLogger({ stateDir: dir, stderr: null, now: () => '2026-08-26T00:00:00.000Z' })
  t.after(() => logger.close())

  logger.log('info', 'claude channel adapter started', { expected_agent: 'CC-Stealth' })
  logger('a bare warning')

  const { body } = readOnlyLog(dir)
  const lines = body.trim().split('\n').map(line => JSON.parse(line))
  assert.deepEqual(lines[0], {
    ts: '2026-08-26T00:00:00.000Z',
    level: 'info',
    message: 'claude channel adapter started',
    expected_agent: 'CC-Stealth',
  })
  assert.deepEqual(lines[1], {
    ts: '2026-08-26T00:00:00.000Z',
    level: 'warn',
    message: 'a bare warning',
  })
})

test('a record after close cannot write to a recycled descriptor', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const logger = createLogger({ stateDir: dir, stderr: null })
  logger.log('info', 'before close')
  logger.close()

  // The fd number is free once closed; without clearing the descriptor these
  // bytes could land in whatever file or socket claimed that number next.
  assert.doesNotThrow(() => logger.log('warn', 'after close'))
  assert.equal(logger.path, null)
  const { body } = readOnlyLog(dir)
  assert.match(body, /before close/)
  assert.doesNotMatch(body, /after close/)
})

test('closing twice is safe', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const logger = createLogger({ stateDir: dir, stderr: null })
  logger.close()
  assert.doesNotThrow(() => logger.close())
})

test('creates the log file unreadable to group and other', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const logger = createLogger({ stateDir: dir, stderr: null })
  t.after(() => logger.close())
  logger.log('info', 'started')

  const { name } = readOnlyLog(dir)
  // Asserting only "no group/other bits" would also accept mode 000.
  assert.equal(statSync(join(dir, name)).mode & 0o777, 0o600)
})

test('refuses a symlinked state directory', t => {
  const base = scratch()
  t.after(() => rmSync(base, { recursive: true, force: true }))
  const real = join(base, 'real')
  const link = join(base, 'link')
  mkdirSync(real)
  symlinkSync(real, link)

  const logger = createLogger({ stateDir: link, stderr: null })
  t.after(() => logger.close())
  logger.log('info', 'started')

  // A symlinked state directory would redirect adapter output to an
  // attacker-chosen path written under the operator's own credentials.
  assert.equal(logger.path, null)
  assert.deepEqual(readdirSync(real), [])
})

test('keeps running when no state directory is configured', () => {
  const logger = createLogger({ stateDir: null, stderr: null })
  assert.equal(logger.path, null)
  assert.doesNotThrow(() => logger.log('warn', 'no file sink'))
  assert.doesNotThrow(() => logger.close())
})

test('still writes to the file when the stderr sink throws', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const exploding = { write() { throw new Error('stderr is gone') } }
  const logger = createLogger({ stateDir: dir, stderr: exploding })
  t.after(() => logger.close())

  // Losing the diagnostic sink must never take down live delivery.
  assert.doesNotThrow(() => logger.log('error', 'still recorded'))
  assert.match(readOnlyLog(dir).body, /still recorded/)
})

test('survives a value that cannot be serialized', t => {
  const dir = scratch()
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const logger = createLogger({ stateDir: dir, stderr: null })
  t.after(() => logger.close())

  const cyclic = {}
  cyclic.self = cyclic
  assert.doesNotThrow(() => logger.log('warn', 'cyclic', { cyclic }))
  assert.match(readOnlyLog(dir).body, /unserializable log record/)
})
