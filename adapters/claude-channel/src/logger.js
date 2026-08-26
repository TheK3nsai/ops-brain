import { randomBytes } from 'node:crypto'
import { closeSync, constants, lstatSync, mkdirSync, openSync, writeSync } from 'node:fs'
import { join } from 'node:path'

const FILE_MODE = 0o600
const DIR_MODE = 0o700
const NAME_ATTEMPTS = 8

// Claude Code owns this adapter's stdio, so stderr is not visible to the
// operator: it is neither shown in the terminal nor recorded in Claude Code's
// MCP log, which captures only its own transport events. Without a file of our
// own, a live channel that never binds is indistinguishable from one that did.
// The Codex launcher redirects its adapter's stderr to a log file; that is not
// possible here because Claude Code spawns this process, so the adapter opens
// its own file instead.
export function createLogger({ stateDir, stderr = process.stderr, now = () => new Date().toISOString() } = {}) {
  let descriptor = stateDir ? openLogFile(stateDir) : null

  const log = (level, message, fields = {}) => {
    // Timestamped because this file is the sole evidence in an attended gate,
    // and each launch writes a separate, randomly named file with no other
    // ordering between them.
    let line
    try {
      line = `${JSON.stringify({ ts: now(), level, message, ...fields })}\n`
    } catch {
      line = `${JSON.stringify({ ts: now(), level, message: 'unserializable log record' })}\n`
    }
    try {
      stderr?.write(line)
    } catch {
      // The operator-visible sink is best effort; never fail the adapter on it.
    }
    if (descriptor === null) return
    try {
      writeAll(descriptor.fd, line)
    } catch {
      // A full or unlinked log file must not stop live delivery.
    }
  }

  log.close = () => {
    if (descriptor === null) return
    const { fd } = descriptor
    // Cleared before the close so a late record cannot write to this number
    // after the descriptor is released and the fd is recycled by another file.
    descriptor = null
    try {
      closeSync(fd)
    } catch {
      // Nothing actionable during shutdown.
    }
  }
  Object.defineProperty(log, 'path', { get: () => descriptor?.path ?? null })

  // A bare logger call keeps the LiveClient's single-argument contract working.
  const warn = message => log('warn', message)
  warn.log = log
  warn.close = log.close
  Object.defineProperty(warn, 'path', { get: () => log.path })
  return warn
}

function writeAll(fd, line) {
  const buffer = Buffer.from(line, 'utf8')
  let written = 0
  // A short write would tear a JSON line and make the record unparseable.
  while (written < buffer.length) {
    written += writeSync(fd, buffer, written, buffer.length - written)
  }
}

function openLogFile(stateDir) {
  try {
    mkdirSync(stateDir, { recursive: true, mode: DIR_MODE })
    // Refuse a symlinked state directory for the same reason the Codex
    // launcher does: it would redirect adapter output to an attacker-chosen
    // path under the operator's own credentials.
    const stats = lstatSync(stateDir)
    if (!stats.isDirectory()) return null
  } catch {
    // Logging is diagnostic. Losing it must never prevent the adapter from
    // starting, or a bad state directory becomes an outage.
    return null
  }
  // O_EXCL refuses to follow a symlinked leaf, so a name collision is a
  // retryable accident rather than a reason to lose the log entirely.
  for (let attempt = 0; attempt < NAME_ATTEMPTS; attempt += 1) {
    const path = join(stateDir, `claude-adapter.${randomBytes(4).toString('hex')}.log`)
    try {
      const flags = constants.O_WRONLY | constants.O_CREAT | constants.O_APPEND | constants.O_EXCL
      return { fd: openSync(path, flags, FILE_MODE), path }
    } catch (error) {
      if (error?.code !== 'EEXIST') return null
    }
  }
  return null
}
