import type { AcpConfigOption } from "../components/types";
import { MODEL_OPTION_ID } from "./runtimeOptions";

const readString = (value: unknown, key: string): string => {
  if (!value || typeof value !== "object") return "";
  const raw = (value as Record<string, unknown>)[key];
  return typeof raw === "string" ? raw : "";
};

export const optionId = (option: AcpConfigOption): string => readString(option, "id");
export const optionName = (option: AcpConfigOption): string => readString(option, "name");

export const optionCurrentValue = (option: AcpConfigOption): string =>
  readString(option, "currentValue") || readString(option, "current_value");

/**
 * Identified by the id the runtime declared, not by matching its name or category.
 *
 * The fuzzy versions of these (`name.includes("reasoning")`, `includes("thinking")`,
 * `includes("thought")`, plus parallel checks on `category`) existed only because parameters
 * did not declare what they were. They are gone: a parameter now arrives with an explicit
 * `kind`, so `runtimeOptions.ts` reads it rather than inferring it, and a runtime can add a
 * parameter without any surface having to recognise its name.
 */
export const isModelOption = (option: AcpConfigOption): boolean =>
  optionId(option) === MODEL_OPTION_ID;

export const isModeOption = (option: AcpConfigOption): boolean => optionId(option) === "mode";
