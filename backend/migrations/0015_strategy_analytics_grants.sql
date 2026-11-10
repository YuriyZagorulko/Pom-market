-- App/worker roles need DML on snapshot table (created after 0001 init grants).

GRANT SELECT, INSERT, UPDATE, DELETE ON strategy_analytics_snapshots TO pommarket_app, pommarket_worker;
