/**
 * The one vocabulary for "what is this session doing". Every view (control center, topbar,
 * sidebar, phone companion) derives its badge from here, so a session never reads "Working" in
 * one place and "Idle" in another. Presentation lives in `components/ui/status-badge.tsx`.
 */
export type SessionStatus =
  | "needs-approval"
  | "needs-answer"
  | "error"
  | "working"
  | "external"
  | "idle";

export type SessionStatusInput = {
  running?: boolean;
  submitting?: boolean;
  /** The provider conversation is being driven outside Jockey (e.g. a terminal). */
  externalActive?: boolean;
  /** Truthy when the last turn failed. */
  error?: boolean | string | null;
  pendingPermissions?: number;
  pendingUserInputs?: number;
};

/** Ordered by what the user must act on first: a decision outranks a failure outranks activity. */
export function sessionStatus(input: SessionStatusInput): SessionStatus {
  if ((input.pendingPermissions ?? 0) > 0) return "needs-approval";
  if ((input.pendingUserInputs ?? 0) > 0) return "needs-answer";
  const busy = !!input.running || !!input.submitting;
  if (input.error && !busy && !input.externalActive) return "error";
  if (busy) return "working";
  if (input.externalActive) return "external";
  return "idle";
}

export function needsUser(status: SessionStatus): boolean {
  return status === "needs-approval" || status === "needs-answer";
}

export function isActive(status: SessionStatus): boolean {
  return status === "working" || status === "external";
}
