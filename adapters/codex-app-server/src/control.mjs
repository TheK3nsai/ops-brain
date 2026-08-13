import {
  assertExactKeys,
  assertMessageBody,
  assertOptionalUuid,
  assertUuid,
  isRecord,
} from './protocol.mjs';

function safeRequestId(command) {
  if (!isRecord(command)) return null;
  try { return assertOptionalUuid(command.request_id, 'command.request_id'); }
  catch { return null; }
}

function validateControl(command) {
  if (!isRecord(command)) throw new Error('control command must be an object');
  if (command.type === 'list_peers') {
    assertExactKeys(command, ['type'], ['request_id'], 'list_peers command');
    return {
      type: command.type,
      requestId: assertOptionalUuid(command.request_id, 'command.request_id'),
    };
  }
  if (command.type === 'send_message') {
    assertExactKeys(
      command,
      ['type', 'to_peer_id', 'body'],
      ['request_id', 'in_reply_to'],
      'send_message command',
    );
    return {
      type: command.type,
      requestId: assertOptionalUuid(command.request_id, 'command.request_id'),
      toPeerId: assertUuid(command.to_peer_id, 'command.to_peer_id'),
      body: assertMessageBody(command.body, 'command.body'),
      inReplyTo: assertOptionalUuid(command.in_reply_to, 'command.in_reply_to'),
    };
  }
  throw new Error('unknown control command');
}

export async function handleControlCommand(command, bridge) {
  const responseRequestId = safeRequestId(command);
  try {
    const validated = validateControl(command);
    if (validated.type === 'list_peers') {
      return {
        ok: true,
        request_id: validated.requestId,
        peers: await bridge.listPeers(),
      };
    }
    const receipt = await bridge.sendMessage({
      toPeerId: validated.toPeerId,
      body: validated.body,
      inReplyTo: validated.inReplyTo,
      requestId: validated.requestId || undefined,
    });
    return { ok: true, request_id: validated.requestId, receipt };
  } catch (error) {
    return { ok: false, request_id: responseRequestId, error: error.message };
  }
}
