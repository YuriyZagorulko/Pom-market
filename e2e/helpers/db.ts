const E2E_DB = "pommarket_e2e";

export function assertE2eDatabaseName(actual: string) {
  if (actual !== E2E_DB) {
    throw new Error(
      `E2E refusal: API database_name is "${actual}"; expected exactly "${E2E_DB}"`,
    );
  }
}

export async function assertE2eApiHealth(apiBase: string) {
  const res = await fetch(`${apiBase}/health`);
  if (!res.ok) {
    throw new Error(`E2E health check failed: HTTP ${res.status}`);
  }
  const body = (await res.json()) as { database_name?: string };
  const name = body.database_name;
  if (!name) {
    throw new Error("E2E health response missing database_name");
  }
  assertE2eDatabaseName(name);
}
