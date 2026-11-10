import Link from "next/link";

export function StrategiesListHeader() {
  return (
    <div className="row" style={{ alignItems: "flex-start", marginBottom: 12 }}>
      <div>
        <h1 style={{ marginBottom: 6 }}>Copy strategies</h1>
        <p className="muted" style={{ margin: 0 }}>
          Each account and source wallet pair has its own budget. A new strategy starts disabled — enable it
          after reviewing settings to begin copying new trades.
        </p>
      </div>
      <Link className="button" href="/strategies/new">
        Create strategy
      </Link>
    </div>
  );
}
