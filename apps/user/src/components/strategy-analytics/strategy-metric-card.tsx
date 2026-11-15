export function MetricCard({
  label,
  value,
  hint,
  className,
}: {
  label: string;
  value: string;
  hint?: string;
  className?: string;
}) {
  return (
    <div className={`metric-card card${className ? ` ${className}` : ""}`}>
      <span className="muted" style={{ fontSize: 13 }}>
        {label}
      </span>
      <b>{value}</b>
      {hint ? (
        <span className="muted" style={{ fontSize: 11 }}>
          {hint}
        </span>
      ) : null}
    </div>
  );
}
