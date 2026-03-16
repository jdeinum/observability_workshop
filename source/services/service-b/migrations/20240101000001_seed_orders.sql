INSERT INTO orders (id, customer_name, created_at) VALUES
  ('20000000-0000-0000-0000-000000000001', 'Alice Demo', now() - interval '2 days'),
  ('20000000-0000-0000-0000-000000000002', 'Bob Demo',   now() - interval '1 day')
ON CONFLICT (id) DO NOTHING;

-- Order 1: 50 items referencing products 1-50
INSERT INTO order_items (order_id, product_id, quantity)
SELECT
    '20000000-0000-0000-0000-000000000001'::uuid,
    ('10000000-0000-0000-0000-' || lpad(i::text, 12, '0'))::uuid,
    (i % 5) + 1
FROM generate_series(1, 50) AS i
ON CONFLICT DO NOTHING;

-- Order 2: 10 items referencing products 1-10
INSERT INTO order_items (order_id, product_id, quantity)
SELECT
    '20000000-0000-0000-0000-000000000002'::uuid,
    ('10000000-0000-0000-0000-' || lpad(i::text, 12, '0'))::uuid,
    (i % 3) + 1
FROM generate_series(1, 10) AS i
ON CONFLICT DO NOTHING;
