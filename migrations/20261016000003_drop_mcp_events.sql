-- MCP Events was parked (branch parked/events-oauth). Drop its schema; the
-- applied 20261016000002 migration stays untouched for checksum stability.
DROP TRIGGER IF EXISTS handoff_created_event ON handoffs;
DROP FUNCTION IF EXISTS enqueue_handoff_created_event();
DROP TABLE IF EXISTS mcp_event_outbox;
DROP TABLE IF EXISTS mcp_event_subscriptions;
