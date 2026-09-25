import type { SafeErrorDto } from "../routes/cockpitContract";
import type { Translator } from "../i18n/messages";

/** The props O05 (`SafeErrorDetail`) renders, resolved from an envelope. */
export interface ResolvedSafeError {
  readonly message: string;
  readonly correlationId: string;
  readonly retryable: boolean;
  /** The envelope's error class, for a surface that offers a class-specific
   * next action (O03 offers "prepare again" on an expired/changed preview).
   * Absent for a non-envelope rejection. Never shown as text. */
  readonly errorCode?: string;
}

/**
 * Whether an IPC rejection is the host's safe error envelope. Anything
 * else (a thrown TypeError, a string, a transport failure) is not, and is
 * rendered as an unknown error with no correlation id rather than being
 * coerced into one -- a fabricated id would be a false trace.
 */
export function isSafeErrorDto(value: unknown): value is SafeErrorDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.errorCode === "string" &&
    typeof candidate.messageKey === "string" &&
    typeof candidate.correlationId === "string" &&
    typeof candidate.retryable === "boolean" &&
    Array.isArray(candidate.messageParams) &&
    Array.isArray(candidate.extensions)
  );
}

const FIELD_KEY_GROUPS = [
  "field",
  "nextStep",
  "label.intent",
  "label.persistedState",
  "label.state",
] as const;

/** A FieldKey parameter value in words: a field name, a next step, an intent or
 * a state, in that order; the value as sent when none matches. */
export function fieldKeyWords(value: string, t: Translator): string {
  for (const group of FIELD_KEY_GROUPS) {
    const words = t.lookup(`${group}.${value}`, {});
    if (words !== null) {
      return words;
    }
  }
  return value;
}

/**
 * One envelope parameter as text, or `null` when it is not a parameter the
 * host could have sent. The envelope crosses a process boundary, so its
 * parameters are checked here rather than trusted: resolving an error must
 * never itself throw. A dropped parameter leaves its slot visible.
 */
function paramEntry(param: unknown, t: Translator): [string, string] | null {
  if (typeof param !== "object" || param === null) {
    return null;
  }
  const { key, value } = param as { key?: unknown; value?: unknown };
  if (typeof key !== "string" || typeof value !== "object" || value === null) {
    return null;
  }
  const typed = value as { type?: unknown; value?: unknown };
  switch (typed.type) {
    case "identifier":
      return typeof typed.value === "string" ? [key, typed.value] : null;
    // A field key names a field, a next step, a lifecycle intent or a state;
    // it is shown in words when the catalog has them, and as sent otherwise
    // rather than not at all.
    case "fieldKey":
      return typeof typed.value === "string" ? [key, fieldKeyWords(typed.value, t)] : null;
    // Written as digits, not as a locale number: these are counts and
    // revisions a person may compare against a log.
    case "unsigned":
      return typeof typed.value === "number" &&
        Number.isSafeInteger(typed.value) &&
        typed.value >= 0
        ? [key, String(typed.value)]
        : null;
    case "boolean":
      return typeof typed.value === "boolean"
        ? [key, typed.value ? t("common.yes") : t("common.no")]
        : null;
    default:
      return null;
  }
}

/** The error sentence and what retrying would do, composed by the catalog so
 * each language orders and joins the two sentences itself. */
function withRetryHint(message: string, retryable: boolean, t: Translator): string {
  return t("error.withHint", {
    message,
    hint: retryable ? t("error.retryHint") : t("error.noRetryHint"),
  });
}

/**
 * Resolve the safe envelope into O05's props, in the language `t` speaks. The
 * raw `messageKey` is never shown (DG3 Error Contract); a key the catalog has
 * no message for renders the generic message, which asserts nothing about
 * what happened. The Correlation ID is kept either way.
 */
export function resolveSafeError(error: SafeErrorDto, t: Translator): ResolvedSafeError {
  const params = Object.fromEntries(
    (error.messageParams as readonly unknown[])
      .map((param) => paramEntry(param, t))
      .filter((entry): entry is [string, string] => entry !== null),
  );
  const body = t.lookup(`safeError.${error.messageKey}`, params) ?? t("error.unknown");
  return {
    message: withRetryHint(body, error.retryable, t),
    correlationId: error.correlationId,
    retryable: error.retryable,
    errorCode: error.errorCode,
  };
}

/**
 * Resolve whatever a load rejected with. A non-envelope rejection has no
 * correlation id; the empty string tells O05 there is nothing to copy.
 */
export function resolveRejection(reason: unknown, t: Translator): ResolvedSafeError {
  if (isSafeErrorDto(reason)) {
    return resolveSafeError(reason, t);
  }
  return {
    message: withRetryHint(t("error.unknown"), true, t),
    correlationId: "",
    retryable: true,
  };
}
