import { useRef, useState } from "react";

/** One pending action per owner; failures stay visible and can be retried. */
export function useAction() {
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (action: () => Promise<unknown>): Promise<boolean> => {
    if (pending.current) return false;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return { busy, error, run, clearError: () => setError(null) };
}
