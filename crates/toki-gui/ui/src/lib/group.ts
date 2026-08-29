export interface Total {
  exePath: string;
  appName: string;
  domain: string | null;
  ms: number;
}

export interface DomainRow {
  domain: string;
  ms: number;
}

export interface AppGroup {
  exePath: string;
  appName: string;
  ms: number;
  /** `null` for a non-browser app — nothing to expand into. */
  domains: DomainRow[] | null;
}

/**
 * Rows already carry both app and domain per DESIGN.md §5 — grouping them under the browser
 * they came from is a display choice here, not a schema one. A row with a domain of `null`
 * folds into "Unresolved" rather than being dropped, so the app total still accounts for
 * every second the daemon recorded.
 */
export function group(totals: Total[]): AppGroup[] {
  const rowsByApp = new Map<string, Total[]>();
  for (const t of totals) {
    const rows = rowsByApp.get(t.exePath);
    if (rows) rows.push(t);
    else rowsByApp.set(t.exePath, [t]);
  }

  const out: AppGroup[] = [];
  for (const rows of rowsByApp.values()) {
    const ms = rows.reduce((sum, r) => sum + r.ms, 0);
    const isBrowser = rows.some((r) => r.domain !== null);
    let domains: DomainRow[] | null = null;
    if (isBrowser) {
      const byDomain = new Map<string, number>();
      for (const r of rows) {
        const key = r.domain ?? "Unresolved";
        byDomain.set(key, (byDomain.get(key) ?? 0) + r.ms);
      }
      domains = [...byDomain.entries()]
        .map(([domain, domainMs]) => ({ domain, ms: domainMs }))
        .sort((a, b) => b.ms - a.ms);
    }
    out.push({ exePath: rows[0].exePath, appName: rows[0].appName, ms, domains });
  }
  return out.sort((a, b) => b.ms - a.ms);
}
