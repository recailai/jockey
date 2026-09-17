import type { AppSession } from "../../components/types";

/** One selectable option row of a popupSelect shell. */
export interface SelectOption {
  readonly id: string;
  readonly label: string;
  /** Optional short marker rendered as a badge beside the label. */
  readonly badge?: string;
  readonly detail?: string;
  readonly active?: boolean;
}

/**
 * Registration for the popupSelect command kind:
 * Displays a floating, searchable list card above the composer.
 */
export interface PopupSelectSpec {
  readonly kind: "popupSelect";
  options(session: AppSession): Promise<readonly SelectOption[]>;
  onSelect(option: SelectOption, session: AppSession): void | Promise<void>;
}

/**
 * Registration for the action command kind:
 * Consumes the trigger token and executes a client-side callback directly.
 */
export interface ActionSpec {
  readonly kind: "action";
  run(session: AppSession): void | Promise<void>;
}

/** The UI behavior of an interactive command. */
export type CommandUiSpec = PopupSelectSpec | ActionSpec;

/**
 * A UI decoration hung on one HOST command:
 * Bare invocation (/model without args) triggers this UI behavior rather than submitting a message turn.
 */
export interface CommandDecoration {
  readonly name: string;
  readonly ui: CommandUiSpec;
}

/**
 * A client-owned command contribution (pure client-side command).
 */
export interface CommandContribution {
  readonly name: string;
  readonly label?: string;
  readonly description?: string;
  readonly ui: CommandUiSpec;
}
