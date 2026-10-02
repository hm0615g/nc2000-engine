// Where the bot's party comes from on the play door when no pool file is
// loaded: the catalog by its draw weights, or the solved Nash mixture. A
// loaded pool file overrides both (app.tsx). Kept per browser like the other
// setup choices; anything unreadable falls back to the catalog.

export type DrawSource = "catalog" | "nash";

const KEY = "nc2000-bot-draw";

export function loadDrawSource(): DrawSource {
  try {
    return localStorage.getItem(KEY) === "nash" ? "nash" : "catalog";
  } catch {
    return "catalog";
  }
}

export function storeDrawSource(source: DrawSource): void {
  try {
    if (source === "nash") localStorage.setItem(KEY, "nash");
    else localStorage.removeItem(KEY);
  } catch {
    /* storage unavailable: the choice still holds this session */
  }
}
