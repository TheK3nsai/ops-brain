#!/usr/bin/env node

import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { createChannelServer } from './channel-server.js'
import { loadConfig } from './config.js'
import { InboundChannelBridge } from './inbound-bridge.js'
import { bindLiveLifecycle } from './lifecycle.js'
import { LiveClient } from './live-client.js'
import { createLogger } from './logger.js'

// Built before the configuration is read: a rejected token file or malformed
// live URL is exactly the kind of startup failure that would otherwise leave
// no trace, because Claude Code discards this process's stderr.
const logger = createLogger({ stateDir: process.env.OPS_BRAIN_LIVE_STATE_DIR?.trim() || null })

async function main() {
  const config = loadConfig()
  const live = new LiveClient(config, { logger })
  const mcp = createChannelServer(live)
  const bridge = new InboundChannelBridge(mcp, live)
  live.on('message', message => bridge.accept(message))
  live.on('ready', peer => logger.log('info', 'live adapter connected', {
    peer_id: peer.peer_id,
    agent_name: peer.agent_name,
    label: config.label,
    live_url: config.url,
  }))
  // A fatal closes the socket, so the disconnect record lands after the error.
  // Without will_reconnect it reads as an ordinary retryable drop and
  // contradicts the terminal line above it.
  live.on('disconnected', ({ code = null, reason = null, willReconnect = false } = {}) => logger.log('warn', 'live adapter disconnected', {
    expected_agent: config.expectedAgent,
    close_code: code,
    reason,
    will_reconnect: willReconnect,
  }))
  live.on('fatal', error => {
    logger.log('error', error.message, {
      expected_agent: config.expectedAgent,
      live_url: config.url,
      retryable: false,
    })
    // The log is the operator's record; this event is the session's. Without
    // it a session that lost its lane keeps looking healthy from the inside,
    // which is the defect the main-launcher integration must not reintroduce.
    void announceLaneLost(mcp, config, error.message)
  })
  const stopLive = bindLiveLifecycle(mcp, live, { warn: logger })

  await mcp.connect(new StdioServerTransport())
  logger.log('info', 'claude channel adapter started', {
    expected_agent: config.expectedAgent,
    label: config.label,
    live_url: config.url,
  })

  const stop = async () => {
    stopLive()
    await mcp.close()
    logger.close()
  }
  process.once('SIGINT', () => void stop())
  process.once('SIGTERM', () => void stop())
}

async function announceLaneLost(mcp, config, reason) {
  try {
    await mcp.notification({
      method: 'notifications/claude/channel',
      params: {
        content: [
          `ops-brain live: LANE LOST for this session (${config.expectedAgent}, label ${config.label}).`,
          `Reason: ${reason}.`,
          'The adapter has stopped and will not reconnect; live sends and receives are unavailable here until the session is relaunched. Handoffs still work.',
        ].join(' '),
        meta: {
          kind: 'lane_status',
          state: 'lost',
          agent_name: config.expectedAgent,
          label: config.label,
          trust: 'adapter_status',
        },
      },
    })
  } catch {
    // The MCP transport may already be gone; the log record above stands.
  }
}

main().catch(error => {
  const message = error instanceof Error ? error.message : 'unknown startup failure'
  logger.log('error', `ops-brain Claude Channel failed: ${message}`, { retryable: false })
  logger.close()
  process.exitCode = 1
})
