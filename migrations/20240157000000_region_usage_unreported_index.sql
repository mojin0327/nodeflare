-- Partial index to speed up the usage-reporting background job.
--
-- list_unreported_workspaces:
--   SELECT DISTINCT workspace_id FROM region_usage
--   WHERE reported_to_stripe = false AND gb_minutes > 0
--
-- list_unreported:
--   SELECT ... FROM region_usage
--   WHERE workspace_id = $1 AND reported_to_stripe = false AND gb_minutes > 0
--
-- Both predicates match this partial index, so the query only scans rows that
-- actually need billing instead of doing a full sequential scan of region_usage.
CREATE INDEX IF NOT EXISTS region_usage_unreported_idx
    ON region_usage (workspace_id)
    WHERE reported_to_stripe = false AND gb_minutes > 0;
