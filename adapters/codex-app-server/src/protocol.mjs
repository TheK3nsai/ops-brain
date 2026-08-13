const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const SLUG_RE = /^[A-Za-z0-9._-]+$/;
const DELIVERY_STATUSES = new Set(['host_accepted', 'routed']);
const ADAPTERS = new Set(['claude_code', 'codex']);
const SOURCE_BINDINGS = new Set(['connection_bound', 'agent_bound_unique_adapter']);

export function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function assertRecord(value, name) {
  if (!isRecord(value)) throw new Error(`${name} must be an object`);
  return value;
}

export function assertExactKeys(value, required, optional = [], name = 'object') {
  assertRecord(value, name);
  const allowed = new Set([...required, ...optional]);
  for (const key of required) {
    if (!Object.hasOwn(value, key)) throw new Error(`${name}.${key} is required`);
  }
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) throw new Error(`${name}.${key} is not allowed`);
  }
}

export function assertUuid(value, name) {
  if (typeof value !== 'string' || !UUID_RE.test(value)) {
    throw new Error(`${name} must be a canonical lowercase UUID`);
  }
  return value;
}

export function assertOptionalUuid(value, name) {
  if (value === undefined || value === null) return null;
  return assertUuid(value, name);
}

export function assertAgentSlug(value, name = 'agent_name') {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 80 || !SLUG_RE.test(value)) {
    throw new Error(`${name} must be a 1-80 byte agent slug`);
  }
  return value;
}

export function assertLabel(value, name = 'label') {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 80 || !SLUG_RE.test(value)) {
    throw new Error(`${name} must be a 1-80 byte label slug`);
  }
  return value;
}

export function assertMessageBody(value, name = 'body') {
  if (typeof value !== 'string' || !value.trim() || Buffer.byteLength(value) > 8000) {
    throw new Error(`${name} must be non-blank text no larger than 8000 bytes`);
  }
  return value;
}

export function validatePeer(value, name = 'peer') {
  assertExactKeys(
    value,
    ['peer_id', 'agent_name', 'adapter', 'label', 'metadata_trust'],
    [],
    name,
  );
  assertUuid(value.peer_id, `${name}.peer_id`);
  assertAgentSlug(value.agent_name, `${name}.agent_name`);
  if (!ADAPTERS.has(value.adapter)) throw new Error(`${name}.adapter is invalid`);
  assertLabel(value.label, `${name}.label`);
  if (value.metadata_trust !== 'self_reported') throw new Error(`${name}.metadata_trust is invalid`);
  return value;
}

export function validateIncomingMessage(value) {
  assertExactKeys(
    value,
    ['message_id', 'reply_peer_id', 'from_agent', 'body', 'trust', 'source_binding'],
    ['in_reply_to'],
    'message',
  );
  assertUuid(value.message_id, 'message.message_id');
  assertUuid(value.reply_peer_id, 'message.reply_peer_id');
  assertAgentSlug(value.from_agent, 'message.from_agent');
  assertMessageBody(value.body, 'message.body');
  assertOptionalUuid(value.in_reply_to, 'message.in_reply_to');
  if (value.trust !== 'untrusted_peer_input') {
    throw new Error('message is missing the mandatory untrusted trust marker');
  }
  if (!SOURCE_BINDINGS.has(value.source_binding)) throw new Error('message.source_binding is invalid');
  return value;
}

export function validateReceipt(value) {
  assertExactKeys(value, ['message_id', 'status', 'detail'], [], 'receipt');
  assertUuid(value.message_id, 'receipt.message_id');
  if (!DELIVERY_STATUSES.has(value.status)) throw new Error('receipt.status is invalid');
  if (typeof value.detail !== 'string' || !value.detail || Buffer.byteLength(value.detail) > 1000) {
    throw new Error('receipt.detail must be 1-1000 bytes');
  }
  return value;
}

export function validateErrorFrame(value) {
  assertExactKeys(value, ['type', 'request_id', 'code', 'message'], [], 'error frame');
  assertOptionalUuid(value.request_id, 'error frame.request_id');
  if (typeof value.code !== 'string' || !value.code || value.code.length > 64 || !SLUG_RE.test(value.code)) {
    throw new Error('error frame.code is invalid');
  }
  if (typeof value.message !== 'string' || !value.message || Buffer.byteLength(value.message) > 2000) {
    throw new Error('error frame.message is invalid');
  }
  return value;
}

export function validateRegisteredFrame(frame, expectedLabel) {
  assertExactKeys(frame, ['type', 'protocol_version', 'peer'], [], 'registered frame');
  if (frame.type !== 'registered' || frame.protocol_version !== 1) {
    throw new Error('registered frame protocol is invalid');
  }
  validatePeer(frame.peer, 'registered frame.peer');
  if (frame.peer.adapter !== 'codex' || frame.peer.label !== expectedLabel) {
    throw new Error('registered peer does not match this adapter');
  }
  return frame.peer;
}

export function validatePeersFrame(frame) {
  assertExactKeys(frame, ['type', 'request_id', 'peers'], [], 'peers frame');
  assertUuid(frame.request_id, 'peers frame.request_id');
  if (!Array.isArray(frame.peers) || frame.peers.length > 256) {
    throw new Error('peers frame.peers must be an array of at most 256 peers');
  }
  frame.peers.forEach((peer, index) => validatePeer(peer, `peers frame.peers[${index}]`));
  return frame.peers;
}

export function validateSendResultFrame(frame) {
  assertExactKeys(frame, ['type', 'request_id', 'receipt'], [], 'send result frame');
  assertUuid(frame.request_id, 'send result frame.request_id');
  return validateReceipt(frame.receipt);
}
