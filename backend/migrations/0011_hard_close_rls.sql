-- Align hard_close RLS with app_is_worker() session pattern (not PG role TO clause).

DROP POLICY IF EXISTS hard_close_ops_worker ON hard_close_operations;
DROP POLICY IF EXISTS hard_close_items_worker ON hard_close_items;
DROP POLICY IF EXISTS hard_close_ops_all ON hard_close_operations;
DROP POLICY IF EXISTS hard_close_items_all ON hard_close_items;

CREATE POLICY hard_close_ops_all ON hard_close_operations FOR ALL
    USING (app_is_worker() OR user_id = app_user_id())
    WITH CHECK (app_is_worker() OR user_id = app_user_id());

CREATE POLICY hard_close_items_all ON hard_close_items FOR ALL
    USING (
        app_is_worker()
        OR EXISTS (
            SELECT 1 FROM hard_close_operations o
            WHERE o.id = hard_close_items.operation_id
              AND o.user_id = app_user_id()
        )
    )
    WITH CHECK (
        app_is_worker()
        OR EXISTS (
            SELECT 1 FROM hard_close_operations o
            WHERE o.id = hard_close_items.operation_id
              AND o.user_id = app_user_id()
        )
    );
