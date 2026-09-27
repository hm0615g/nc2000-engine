import type { EvaluationRun } from "./evaluate-core";

function openStore(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("nc2000-evaluations", 1);
    req.onupgradeneeded = () =>
      req.result.createObjectStore("runs", { keyPath: "id" });
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
    req.onblocked = () =>
      reject(new Error("保存用データベースが別のタブで使用されています。"));
  });
}
export async function saveRun(run: EvaluationRun): Promise<void> {
  const db = await openStore();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction("runs", "readwrite");
      tx.objectStore("runs").put(run);
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
      tx.onerror = () => reject(tx.error);
    });
  } finally {
    db.close();
  }
}
export async function loadRuns(): Promise<EvaluationRun[]> {
  const db = await openStore();
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction("runs", "readonly");
      const req = tx.objectStore("runs").getAll();
      tx.oncomplete = () =>
        resolve(
          (req.result as EvaluationRun[])
            .filter((r) => r.version === 1)
            .sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
        );
      tx.onabort = () => reject(tx.error);
      req.onerror = () => reject(req.error);
    });
  } finally {
    db.close();
  }
}
