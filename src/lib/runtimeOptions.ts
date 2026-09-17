import type { AcpConfigOption, ConfigOptionGroup, ConfigOptionValue } from "../components/types";
import { flattenConfigValues } from "../components/types";

/** The parameter every other one is scoped against, so it stays special in both surfaces. */
export const MODEL_OPTION_ID = "model";

/** Ids the backend declares for the reasoning axis. A model restricts this parameter through
 *  its own `effortLevels`, and that restriction arrives on the model value rather than on the
 *  parameter, so the link has to be named somewhere. Listing the ids is bounded and checkable;
 *  matching option *names* against "reasoning"/"thinking"/"thought" — which is what this
 *  replaces — silently caught unrelated parameters. Delete this once the backend scopes a
 *  parameter's values per model. */
const MODEL_RESTRICTED_IDS = new Set(["effort", "reasoning_effort", "thinking_level"]);

/** Keys a value may still be stored under from before the runtime declared its own id. The
 *  composer wrote every runtime's reasoning level as `effort`, so a Codex persona configured
 *  then holds `effort` while the runtime now declares `reasoning_effort`. Read-only shim. */
const LEGACY_KEYS: Record<string, string[]> = {
  reasoning_effort: ["effort"],
  thinking_level: ["effort"],
};

export type RuntimeOptionValue = {
  value: string;
  name: string;
  description: string;
  /** Present on model values: what that model alone supports. */
  effortLevels: string[];
  defaultEffort: string;
  isDefault: boolean;
  oneMillion: boolean;
  supportsFast: boolean;
};

export type RuntimeOption = {
  id: string;
  name: string;
  kind: "select" | "toggle";
  description: string;
  /** What the runtime uses when nothing is chosen, as reported by the CLI itself. */
  runtimeDefault: string;
  /** Empty means it applies to every model. */
  appliesToModels: string[];
  values: RuntimeOptionValue[];
};

const asStr = (v: unknown): string => (typeof v === "string" ? v : "");
const asBool = (v: unknown): boolean => v === true;
const asStrArray = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];

const readValue = (raw: ConfigOptionValue): RuntimeOptionValue => {
  const extra = raw as unknown as Record<string, unknown>;
  return {
    value: raw.value,
    name: asStr(raw.name) || raw.value,
    description: asStr(raw.description),
    effortLevels: asStrArray(extra.effortLevels),
    defaultEffort: asStr(extra.defaultEffort),
    isDefault: asBool(extra.isDefault),
    oneMillion: asBool(extra.oneMillion),
    supportsFast: asBool(extra.supportsFast),
  };
};

/**
 * The parameters a runtime declared. `kind` is read from the declaration; `type` is only
 * consulted for catalogs persisted before runtimes declared a kind, which is reading an older
 * revision of the same contract rather than guessing at one.
 */
export const readRuntimeOptions = (opts: AcpConfigOption[]): RuntimeOption[] =>
  (opts ?? [])
    .map((opt) => {
      const raw = opt as unknown as Record<string, unknown>;
      const id = asStr(raw.id);
      if (!id) return null;
      const declared = asStr(raw.kind);
      const kind: "select" | "toggle" =
        declared === "toggle" || declared === "select"
          ? declared
          : asStr(raw.type) === "toggle"
            ? "toggle"
            : "select";
      const source = (raw.values ?? raw.options ?? []) as
        | ConfigOptionValue[]
        | ConfigOptionGroup[];
      return {
        id,
        name: asStr(raw.name) || id,
        kind,
        description: asStr(raw.description),
        runtimeDefault: asStr(raw.default),
        appliesToModels: asStrArray(raw.appliesToModels),
        values: flattenConfigValues(source)
          .filter((v) => typeof v?.value === "string" && v.value.length > 0)
          .map(readValue),
      };
    })
    .filter((o): o is RuntimeOption => o !== null);

export const findModelOption = (opts: RuntimeOption[]): RuntimeOption | undefined =>
  opts.find((o) => o.id === MODEL_OPTION_ID);

/** Everything except the model, which both surfaces render as a first-class row. */
export const otherRuntimeOptions = (opts: RuntimeOption[]): RuntimeOption[] =>
  opts.filter((o) => o.id !== MODEL_OPTION_ID);

/** A parameter scoped to specific models shows only while one of them is in effect. */
export const appliesToModel = (option: RuntimeOption, modelId: string): boolean =>
  option.appliesToModels.length === 0 ||
  (modelId !== "" && option.appliesToModels.includes(modelId));

/**
 * The values a parameter offers given the model in effect. A model that declares its own
 * supported levels narrows the runtime-wide list — offering a level the selected model cannot
 * run would be a silent lie.
 */
export const valuesForModel = (
  option: RuntimeOption,
  model: RuntimeOptionValue | undefined,
): RuntimeOptionValue[] => {
  const levels = model?.effortLevels ?? [];
  if (levels.length === 0 || !MODEL_RESTRICTED_IDS.has(option.id)) return option.values;
  const declared = new Map(option.values.map((v) => [v.value, v]));
  return levels.map((level) => declared.get(level) ?? { ...emptyValue(level) });
};

const emptyValue = (value: string): RuntimeOptionValue => ({
  value,
  name: value,
  description: "",
  effortLevels: [],
  defaultEffort: "",
  isDefault: false,
  oneMillion: false,
  supportsFast: false,
});

/** The stored value for a parameter, honouring keys written before ids were declared. */
export const storedValue = (config: Record<string, string>, option: RuntimeOption): string => {
  const own = config[option.id];
  if (own) return own;
  for (const legacy of LEGACY_KEYS[option.id] ?? []) {
    if (config[legacy]) return config[legacy];
  }
  return "";
};

/**
 * What the parameter will actually use when nothing is chosen: the model's own default where
 * the model declares one, else the runtime's declared default.
 */
export const effectiveDefault = (
  option: RuntimeOption,
  model: RuntimeOptionValue | undefined,
): string => {
  if (MODEL_RESTRICTED_IDS.has(option.id) && model?.defaultEffort) return model.defaultEffort;
  return option.runtimeDefault;
};
