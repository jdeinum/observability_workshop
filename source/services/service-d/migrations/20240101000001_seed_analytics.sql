INSERT INTO analytics_events (order_id, event_type, log_line)
SELECT
    ('20000000-0000-0000-0000-' || lpad(n::text, 12, '0'))::uuid,
    'order_processed',
    'order_id: 20000000-0000-0000-0000-' || lpad(n::text, 12, '0') || ' status=completed'
FROM generate_series(1, 2) AS n
ON CONFLICT DO NOTHING;
