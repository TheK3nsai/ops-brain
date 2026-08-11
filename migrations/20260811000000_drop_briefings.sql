-- v5.0 — briefings are stateless delivery output, not durable bus state.
-- Historical rows had no supported read surface and duplicated the external
-- delivery archive, so retaining them only created silent accumulation.
DROP TABLE IF EXISTS briefings;
