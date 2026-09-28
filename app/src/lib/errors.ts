import type { ApiError, ApiErrorCode } from "@/ipc/api";
import { strings } from "./strings";
import { formatHm, localOffsetMinutes, minutesUntil, parseIso } from "./time";

const CODES: ReadonlySet<ApiErrorCode> = new Set<ApiErrorCode>([
  "camera_locked",
  "auth_failed",
  "third_party_compat_off",
  "playback_busy",
  "stream_limit",
  "offline",
  "privacy_mode",
  "unsupported",
  "not_found",
  "invalid_input",
  "internal",
]);

export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    CODES.has((value as { code: ApiErrorCode }).code)
  );
}

/** Normalises anything a promise rejected with into an `ApiError`. */
export function toApiError(value: unknown): ApiError {
  if (isApiError(value)) return value;
  if (value instanceof Error) return { code: "internal", message: value.message };
  return { code: "internal", message: typeof value === "string" ? value : "Unknown error" };
}

export interface ErrorCopy {
  title: string;
  body: string;
  hint?: string;
}

/**
 * Friendly copy for an API error. `invalid_input` and `internal` surface the backend's own
 * message, which is written for people; other codes use fixed copy.
 */
export function describeError(
  error: ApiError,
  options: { now?: number; offsetMinutes?: number } = {},
): ErrorCopy {
  const now = options.now ?? Date.now();
  const e = strings.errors;
  switch (error.code) {
    case "camera_locked": {
      const mins = error.retryAt ? minutesUntil(error.retryAt, now) : 30;
      const at = error.retryAt
        ? formatHm(parseIso(error.retryAt), options.offsetMinutes ?? localOffsetMinutes(now))
        : "";
      return { title: e.camera_locked.title, body: e.camera_locked.body(mins, at) };
    }
    case "auth_failed":
      return { title: e.auth_failed.title, body: e.auth_failed.body, hint: e.auth_failed.hint };
    case "invalid_input":
      return { title: e.invalid_input.title, body: error.message || e.invalid_input.body };
    case "internal":
      return { title: e.internal.title, body: error.message || e.internal.body };
    default:
      return { title: e[error.code].title, body: e[error.code].body };
  }
}

/** Retry policy for queries: errors that need the user to act are never retried. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (failureCount >= 2) return false;
  const code = toApiError(error).code;
  return code === "internal" || code === "offline";
}
