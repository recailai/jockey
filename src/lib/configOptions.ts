import type { AcpConfigOption } from "../components/types";

const readString = (value: unknown, key: string): string => {
  if (!value || typeof value !== "object") return "";
  const raw = (value as Record<string, unknown>)[key];
  return typeof raw === "string" ? raw : "";
};

export const optionId = (option: AcpConfigOption): string => readString(option, "id");
export const optionName = (option: AcpConfigOption): string => readString(option, "name");
export const optionCategory = (option: AcpConfigOption): string => readString(option, "category");

export const optionCurrentValue = (option: AcpConfigOption): string =>
  readString(option, "currentValue") || readString(option, "current_value");

const norm = (value: string | undefined | null): string =>
  (value ?? "").trim().toLowerCase().replace(/[-\s]+/g, "_");

export const isModelOption = (option: AcpConfigOption): boolean => {
  const id = norm(optionId(option));
  const cat = norm(optionCategory(option));
  const name = norm(optionName(option));
  return id === "model" || cat === "model" || name === "model";
};

export const isModeOption = (option: AcpConfigOption): boolean => {
  const id = norm(optionId(option));
  const cat = norm(optionCategory(option));
  const name = norm(optionName(option));
  return id === "mode" || cat === "mode" || name === "mode";
};

export const isEffortOption = (option: AcpConfigOption): boolean => {
  const id = norm(optionId(option));
  const cat = norm(optionCategory(option));
  const name = norm(optionName(option));
  return (
    id === "effort" ||
    id === "reasoning_effort" ||
    id === "thinking_effort" ||
    id === "thought_level" ||
    cat === "effort" ||
    cat === "reasoning_effort" ||
    cat === "thinking_effort" ||
    cat === "thought_level" ||
    name.includes("effort") ||
    name.includes("reasoning") ||
    name.includes("thinking") ||
    name.includes("thought")
  );
};
