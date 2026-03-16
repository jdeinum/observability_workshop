INSERT INTO products (id, name, price, category)
SELECT
    ('10000000-0000-0000-0000-' || lpad(i::text, 12, '0'))::uuid,
    'Product ' || i,
    round((random() * 90 + 10)::numeric, 2),
    CASE WHEN i % 3 = 0 THEN 'widgets'
         WHEN i % 3 = 1 THEN 'gadgets'
         ELSE 'accessories' END
FROM generate_series(1, 50) AS i
ON CONFLICT (id) DO NOTHING;
