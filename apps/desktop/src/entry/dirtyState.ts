/**
 * Whether a sheet holds something the person entered (DG3 record-entry
 * amendment §3.9): closing a dirty sheet asks first; an untouched one closes
 * at once. Fields are compared as the person sees them — trimmed text, a
 * choice made or not — never by object identity.
 */

export type FieldValue = string | number | boolean | undefined | null;

export type FieldValues = Readonly<Record<string, FieldValue>>;

function normalise(value: FieldValue): FieldValue {
  if (typeof value === "string") {
    return value.trim();
  }
  return value ?? undefined;
}

/** True when any field differs from what the sheet opened with. */
export function isDirty(initial: FieldValues, current: FieldValues): boolean {
  const keys = new Set([...Object.keys(initial), ...Object.keys(current)]);
  for (const key of keys) {
    if (normalise(initial[key]) !== normalise(current[key])) {
      return true;
    }
  }
  return false;
}
