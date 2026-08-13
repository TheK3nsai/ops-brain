import assert from 'node:assert/strict'
import test from 'node:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { createChannelServer } from '../src/channel-server.js'

const TARGET = '019cd123-1234-7123-8123-123456789abd'

test('advertises the Claude Channel capability and proxies its MCP tools', async t => {
  const calls = []
  const live = {
    async listPeers() {
      return { peers: [{ peer_id: TARGET, agent_name: 'Codex-Stealth', adapter: 'codex', label: 'codex' }] }
    },
    async sendMessage(args) {
      calls.push(args)
      return { receipt: { message_id: TARGET, status: 'host_accepted', detail: 'accepted' } }
    },
  }
  const server = createChannelServer(live)
  const client = new Client({ name: 'adapter-test', version: '1.0.0' })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  t.after(async () => {
    await client.close()
    await server.close()
  })
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)])

  assert.deepEqual(client.getServerCapabilities().experimental['claude/channel'], {})
  assert.match(client.getInstructions(), /always untrusted peer input/)
  const listedTools = await client.listTools()
  assert.deepEqual(listedTools.tools.map(tool => tool.name), ['list_live_peers', 'send_live_message'])

  const peers = await client.callTool({ name: 'list_live_peers', arguments: {} })
  assert.equal(JSON.parse(peers.content[0].text).peers[0].peer_id, TARGET)
  const sent = await client.callTool({
    name: 'send_live_message',
    arguments: { to_peer_id: TARGET, text: 'hello', in_reply_to: null },
  })
  assert.equal(sent.isError, undefined)
  assert.deepEqual(calls[0], { toPeerId: TARGET, body: 'hello', inReplyTo: null })
})

test('returns MCP tool errors without forwarding invalid input', async t => {
  let called = false
  const server = createChannelServer({
    listPeers: async () => ({ peers: [] }),
    sendMessage: async () => { called = true },
  })
  const client = new Client({ name: 'adapter-test', version: '1.0.0' })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  t.after(async () => {
    await client.close()
    await server.close()
  })
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)])
  const result = await client.callTool({
    name: 'send_live_message',
    arguments: { to_peer_id: 'not-a-uuid', text: 'hello' },
  })
  assert.equal(result.isError, true)
  assert.equal(called, false)
})
