#!/usr/bin/env node

import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { createChannelServer } from './channel-server.js'
import { loadConfig } from './config.js'
import { InboundChannelBridge } from './inbound-bridge.js'
import { LiveClient } from './live-client.js'

async function main() {
  const config = loadConfig()
  const live = new LiveClient(config)
  const mcp = createChannelServer(live)
  const bridge = new InboundChannelBridge(mcp, live)
  live.on('message', message => bridge.accept(message))

  await mcp.connect(new StdioServerTransport())
  live.start()

  const stop = async () => {
    live.stop()
    await mcp.close()
  }
  process.once('SIGINT', () => void stop())
  process.once('SIGTERM', () => void stop())
}

main().catch(error => {
  const message = error instanceof Error ? error.message : 'unknown startup failure'
  process.stderr.write(`ops-brain Claude Channel failed: ${message}\n`)
  process.exitCode = 1
})
