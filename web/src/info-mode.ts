export type InfoMode = "open" | "blind";
export type Door = "open" | "blind" | "nash" | "solver" | "evaluate";

export function readDoor(search?: string): Door {
  const query = search ?? (typeof location === "undefined" ? "" : location.search);
  const params = new URLSearchParams(query);
  for (const door of ["evaluate", "solver", "nash", "blind"] as const) {
    if (isOpen(params.get(door))) return door;
  }
  return "open";
}

export function infoModeOf(door: Door): InfoMode {
  return door === "open" ? "open" : "blind";
}

export function readInfoMode(search?: string): InfoMode {
  return infoModeOf(readDoor(search));
}

function isOpen(v: string | null): boolean {
  if (v === null) return false;
  const t = v.trim().toLowerCase();
  return t !== "0" && t !== "false";
}
