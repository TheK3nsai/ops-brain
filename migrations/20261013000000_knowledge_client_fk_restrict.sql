-- Deleting a client must not turn its knowledge global.
--
-- knowledge.client_id was ON DELETE SET NULL, and client_id IS NULL is the
-- definition of global knowledge: it passes the cross-client gate for every
-- caller and is the one class an exclude_client_data token is served. So a
-- client delete would have silently published that client's rows fleet-wide,
-- restricted tokens included.
--
-- RESTRICT makes the delete fail instead. An operator retiring a client
-- deletes or deliberately re-scopes its knowledge first.

ALTER TABLE knowledge DROP CONSTRAINT knowledge_client_id_fkey;

ALTER TABLE knowledge
    ADD CONSTRAINT knowledge_client_id_fkey
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE RESTRICT;
