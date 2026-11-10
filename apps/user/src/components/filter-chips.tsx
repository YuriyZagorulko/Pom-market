"use client";

export function FilterChips({
  chips,
  onRemove,
  onClear,
}: {
  chips: Array<{ name: string; label: string; value: string }>;
  onRemove: (name: string) => void;
  onClear: () => void;
}) {
  if (!chips.length) return null;
  return (
    <div className="filter-chips">
      {chips.map((chip) => (
        <button
          key={chip.name}
          type="button"
          className="chip"
          onClick={() => onRemove(chip.name)}
          title={`Remove ${chip.label}`}
        >
          <span>{chip.label}:</span> {chip.value.length > 40 ? `${chip.value.slice(0, 40)}…` : chip.value}
          <span aria-hidden> ×</span>
        </button>
      ))}
      <button type="button" className="ghost chip-clear" onClick={onClear}>
        Clear all
      </button>
    </div>
  );
}
