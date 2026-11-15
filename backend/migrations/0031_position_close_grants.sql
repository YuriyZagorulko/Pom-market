-- Worker inserts close snapshots; app reads them for Activity.

GRANT SELECT, INSERT ON position_close_events TO pommarket_app, pommarket_worker;
