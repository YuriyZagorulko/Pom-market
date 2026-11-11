import type { InputHTMLAttributes } from "react";
import { formatNumericInputDisplay } from "@pom/format";

type NumericInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "defaultValue" | "type"
> & {
  defaultValue?: string | number | null;
  type?: "text";
};

/** Uncontrolled numeric field: formats API defaults without coercing through JS Number. */
export function NumericInput({ defaultValue, type, ...rest }: NumericInputProps) {
  return (
    <input
      {...rest}
      type={type ?? "text"}
      inputMode={rest.inputMode ?? "decimal"}
      autoComplete="off"
      defaultValue={formatNumericInputDisplay(defaultValue)}
    />
  );
}
