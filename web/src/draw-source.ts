// Where the bot's party comes from on the play door when no pool file is
// loaded: the catalog by its draw weights, the new solved Nash mixture, or
// the preserved v3 Nash mixture. A loaded pool file overrides all of them
// (app.tsx). Kept per browser like the other setup choices; anything
// unreadable falls back to the catalog.

export type DrawSource = "catalog" | "nash" | "nash-v3";
export type NashVariant = "new" | "v3";

const KEY = "nc2000-bot-draw";

/** Read draw source from URL search query (e.g. ?nash, ?nash=v3).
 * Returns null if the query does not specify a draw source. */
export function readDrawSourceFromQuery(search?: string): DrawSource | null {
  const query = search ?? (typeof location === "undefined" ? "" : location.search);
  const params = new URLSearchParams(query);
  const nash = params.get("nash");
  if (nash === null) return null;
  const v = nash.trim().toLowerCase();
  if (v === "0" || v === "false" || v === "catalog") return "catalog";
  if (v === "v3" || v === "old") return "nash-v3";
  return "nash";
}

/** Which Nash variant was requested in the URL, defaulting to "new". */
export function readNashVariant(search?: string): NashVariant {
  return readDrawSourceFromQuery(search) === "nash-v3" ? "v3" : "new";
}

export function loadDrawSource(): DrawSource {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "nash" || v === "nash-v3") return v;
    return "catalog";
  } catch {
    return "catalog";
  }
}

/** Determine initial draw source: URL query takes precedence, then stored choice. */
export function initialDrawSource(search?: string): DrawSource {
  const fromQuery = readDrawSourceFromQuery(search);
  if (fromQuery !== null) return fromQuery;
  return loadDrawSource();
}

export function storeDrawSource(source: DrawSource): void {
  try {
    if (source === "nash" || source === "nash-v3")
      localStorage.setItem(KEY, source);
    else localStorage.removeItem(KEY);
  } catch {
    /* storage unavailable: the choice still holds this session */
  }
}
