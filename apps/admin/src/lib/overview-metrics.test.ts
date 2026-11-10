import { describe, expect, it } from "vitest";
import { countMetrics, overviewMetricLabel, parseKillSwitch } from "./overview-metrics";

describe("overview metrics", () => {
  it("labels kill switch for display", () => {
    expect(overviewMetricLabel("kill_switch")).toBe("kill switch");
    expect(overviewMetricLabel("users")).toBe("Users");
  });

  it("parses kill switch enabled flag", () => {
    expect(parseKillSwitch({ enabled: false })).toBe(false);
    expect(parseKillSwitch({ enabled: true })).toBe(true);
    expect(parseKillSwitch('{"enabled":true}')).toBe(true);
  });

  it("formats count metrics without kill_switch json", () => {
    const rows = countMetrics({
      users: 3,
      accounts: 2,
      kill_switch: { enabled: false },
    });
    expect(rows.find((r) => r.key === "users")?.value).toBe(3);
    expect(rows.some((r) => r.key === "kill_switch")).toBe(false);
  });
});
