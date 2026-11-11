export type CapitalFundingStatus = "not_funded" | "partially_funded" | "funded";

export function capitalFundingLabel(status: CapitalFundingStatus): string {
  switch (status) {
    case "funded":
      return "Funded";
    case "partially_funded":
      return "Partially funded";
    default:
      return "Not funded";
  }
}

export function capitalFundingBadgeClass(status: CapitalFundingStatus): string {
  switch (status) {
    case "funded":
      return "status-badge capital-funded";
    case "partially_funded":
      return "status-badge capital-partial";
    default:
      return "status-badge capital-not-funded";
  }
}

export const ENABLED_UNFUNDED_WARNING =
  "Strategy is enabled, but its capital has not been funded. Paper copy trades are blocked until capital is allocated.";
