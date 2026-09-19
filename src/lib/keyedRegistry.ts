export type RegistryEntry<T> = {
  key: string;
  value: T;
  priority?: number;
};

export type KeyedRegistry<T> = {
  register: (entry: RegistryEntry<T>) => void;
  get: (key: string) => T | undefined;
  has: (key: string) => boolean;
  entries: () => Array<RegistryEntry<T>>;
};

export function createKeyedRegistry<T>(): KeyedRegistry<T> {
  const values = new Map<string, RegistryEntry<T>>();

  const register = (entry: RegistryEntry<T>) => {
    const key = entry.key.trim().toLowerCase();
    if (!key) throw new Error("registry key cannot be empty");
    const normalized = { ...entry, key, priority: entry.priority ?? 0 };
    const existing = values.get(key);
    if (existing && normalized.priority === existing.priority) {
      throw new Error(`duplicate registry key: ${key}`);
    }
    if (!existing || normalized.priority > (existing.priority ?? 0)) {
      values.set(key, normalized);
    }
  };

  return {
    register,
    get: (key) => values.get(key.trim().toLowerCase())?.value,
    has: (key) => values.has(key.trim().toLowerCase()),
    entries: () => [...values.values()],
  };
}
